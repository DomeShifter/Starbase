//! File › Version history, as in Google Docs: a preview mode (the shell swaps its title,
//! menus and toolbar for [`History::top_bar`] and shows the document read-only) with a panel on
//! the right listing the document's
//! versions kept by the Starbase server (newest first: when, who saved it, its name), "Name
//! current version", and for an earlier version: show it (read-only, in Viewing), "Restore this
//! version" (the server keeps the current one as a version first, so nothing is lost) and "Make
//! a copy". Only editors see the history; the server refuses viewers, as Google does.
//!
//! This module is the panel and the naming dialog only: what they ask for ([`Ask`]) is carried
//! out by the shell through the server link (`server::Link`: `GET api/files/<id>/history`,
//! `GET …/versions/<v>`, `POST …/versions/<v>/name?name=`, `POST …/versions/<v>/restore`).

use egui::{Align, Color32, Layout, RichText, Ui, vec2};
use serde_json::Value;

use crate::shell::{BLUE, DIVIDER, ON, TEXT, TEXT_DIM};

/// The id the server gives the file as it is now.
pub const CURRENT: &str = "current";

/// What the panel says when there is no server to keep versions.
pub const NO_SERVER: &str = "Version history needs a Starbase server: open the document from the home screen.";

const ERROR: Color32 = Color32::from_rgb(0xB3, 0x26, 0x1E);
const WIDTH: f32 = 300.0;

/// One version: [`CURRENT`] or the server's id for an earlier one.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Version {
    pub id: String,
    /// When this content was saved (Unix seconds).
    pub saved: i64,
    /// Who saved it (username; empty when unknown).
    pub author: String,
    /// The name given to it, if any.
    pub name: String,
    /// It was made by restoring the version saved then (Unix seconds).
    pub restored_from: Option<i64>,
}

impl Version {
    #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code, reason = "the server link's and tests only"))]
    fn from_json(id: &str, v: &Value) -> Self {
        Version {
            id: id.to_string(),
            saved: v["saved"].as_i64().unwrap_or(0),
            author: v["author"].as_str().unwrap_or("").to_string(),
            name: v["name"].as_str().unwrap_or("").to_string(),
            restored_from: v["restored_from"].as_i64(),
        }
    }
}

/// The server's history reply: the current version first, then the earlier ones, newest first.
#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code, reason = "the server link's and tests only"))]
pub fn parse(v: &Value) -> Vec<Version> {
    let mut out = vec![Version::from_json(CURRENT, &v["current"])];
    for e in v["versions"].as_array().into_iter().flatten() {
        if let Some(id) = e["v"].as_str() {
            out.push(Version::from_json(id, e));
        }
    }
    out
}

/// What the panel asks the shell to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    /// Fetch the list.
    Load,
    /// Show this version (read-only), or [`CURRENT`] to go back to the document.
    Show(String),
    Restore(String),
    Copy(String),
    /// Name a version ([`CURRENT`] for the file as it is now); an empty name removes it.
    Name(String, String),
    /// The panel closed: back to the document.
    Close,
}

#[derive(Default)]
pub struct History {
    pub open: bool,
    /// `None` while loading.
    pub list: Option<Vec<Version>>,
    pub error: Option<String>,
    /// The version shown ([`CURRENT`] = the document itself).
    pub selected: String,
    /// The naming dialog: which version, the name typed, and whether it takes the keyboard next.
    naming: Option<(String, String, bool)>,
    asks: Vec<Ask>,
}

impl History {
    /// File › Version history › See version history (Ctrl+Alt+Shift+H).
    pub fn show(&mut self) {
        *self = History { open: true, selected: CURRENT.to_string(), asks: vec![Ask::Load], ..History::default() };
    }

    /// File › Version history › Name current version.
    pub fn name_current(&mut self) {
        let name = self.list.as_ref().and_then(|l| l.first()).map(|v| v.name.clone()).unwrap_or_default();
        self.naming = Some((CURRENT.to_string(), name, true));
    }

    pub fn naming(&self) -> bool {
        self.naming.is_some()
    }

    /// The server's answer to [`Ask::Load`].
    pub fn loaded(&mut self, list: Result<Vec<Version>, String>) {
        match list {
            Ok(l) => {
                if !l.iter().any(|v| v.id == self.selected) {
                    self.selected = CURRENT.to_string();
                }
                self.list = Some(l);
                self.error = None;
            }
            Err(e) => {
                self.list = Some(Vec::new());
                self.error = Some(e);
            }
        }
    }

    /// What the panel and dialog asked for since last time.
    pub fn take_asks(&mut self) -> Vec<Ask> {
        std::mem::take(&mut self.asks)
    }

    fn close(&mut self) {
        self.open = false;
        self.selected = CURRENT.to_string();
        self.asks.push(Ask::Close);
    }

    /// The selected version's date, for the top bar ("Version history" while the list loads).
    fn shown_title(&self) -> String {
        self.list.iter().flatten().find(|v| v.id == self.selected).map_or_else(|| "Version history".to_string(), |v| when(v.saved))
    }

    /// Google's history bar, drawn in place of the title, menus and toolbar while the history is
    /// open: ← back to the document, the shown version's date, and for an earlier version "Make
    /// a copy" and the filled "Restore this version".
    pub fn top_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.set_min_height(48.0);
            let back = match crate::icons::icon_px("arrow_back", 24.0) {
                Some(img) => egui::Button::image(img),
                None => egui::Button::new("←"),
            };
            let r = ui.add(back.frame(false).min_size(vec2(40.0, 40.0)));
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Back"));
            if r.on_hover_text("Back to the document").clicked() {
                self.close();
            }
            ui.add_space(8.0);
            ui.label(RichText::new(self.shown_title()).size(22.0).color(TEXT));
            if self.selected != CURRENT {
                let id = self.selected.clone();
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if crate::looks::pill(ui, "Restore this version", true).clicked() {
                        self.asks.push(Ask::Restore(id.clone()));
                    }
                    if crate::looks::pill(ui, "Make a copy", false).clicked() {
                        self.asks.push(Ask::Copy(id.clone()));
                    }
                });
            }
        });
    }

    /// The versions list on the right, when open. Call before the page is drawn.
    pub fn panel(&mut self, ui: &mut Ui) {
        if !self.open {
            return;
        }
        egui::Panel::right("stardocs_versions")
            .resizable(false)
            .exact_size(WIDTH)
            .frame(egui::Frame::NONE.fill(Color32::WHITE).stroke(egui::Stroke::new(1.0, DIVIDER)).inner_margin(egui::Margin::symmetric(0, 16)))
            .show(ui, |ui| self.contents(ui));
    }

    fn contents(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(RichText::new("Version history").size(18.0).color(TEXT));
        });
        ui.add_space(12.0);
        if let Some(e) = &self.error {
            ui.horizontal_wrapped(|ui| {
                ui.add_space(16.0);
                ui.label(RichText::new(e).size(13.0).color(ERROR));
            });
        }
        let Some(list) = self.list.clone() else {
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                ui.label(RichText::new("Loading…").size(13.0).color(TEXT_DIM));
            });
            return;
        };
        if list.len() == 1 && self.error.is_none() {
            ui.horizontal_wrapped(|ui| {
                ui.add_space(16.0);
                ui.label(RichText::new("Earlier versions appear here once the document has been saved again.").size(12.0).color(TEXT_DIM));
            });
            ui.add_space(6.0);
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for v in &list {
                self.row(ui, v);
            }
        });
    }

    /// One version, as Google lists them: its date (or name) in bold, "Current version" in
    /// italics, who saved it after a coloured dot; the shown one on a light blue band.
    fn row(&mut self, ui: &mut Ui, v: &Version) {
        let selected = v.id == self.selected;
        let width = ui.available_width();
        let when = when(v.saved);
        let (rect, resp) = ui.allocate_exact_size(vec2(width, row_height(v)), egui::Sense::click());
        let title = if v.name.is_empty() { when.clone() } else { v.name.clone() };
        let mut label = title.clone();
        if v.id == CURRENT {
            label.push_str(", Current version");
        }
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{label} {}", v.author)));
        let p = ui.painter();
        let fill = if selected {
            ON
        } else if resp.hovered() {
            crate::theme::MENU_HOVER
        } else {
            Color32::TRANSPARENT
        };
        p.rect_filled(rect, 0.0, fill);
        let x = rect.min.x + 24.0;
        let mut y = rect.min.y + 10.0;
        let wrap = width - 40.0;
        let line = |y: &mut f32, text: String, font: egui::FontId, colour: Color32, dx: f32| {
            let g = p.layout(text, font, colour, wrap);
            let h = g.size().y;
            p.galley(egui::pos2(x + dx, *y), g, colour);
            *y += h + 2.0;
        };
        line(&mut y, title, egui::FontId::proportional(14.0), TEXT, 0.0);
        if v.id == CURRENT {
            line(&mut y, "Current version".into(), egui::FontId::proportional(12.0), TEXT_DIM, 0.0);
        }
        if !v.name.is_empty() {
            line(&mut y, when, egui::FontId::proportional(12.0), TEXT_DIM, 0.0);
        }
        if let Some(from) = v.restored_from {
            line(&mut y, format!("Restored from {}", self::when(from)), egui::FontId::proportional(12.0), TEXT_DIM, 0.0);
        }
        if !v.author.is_empty() {
            p.circle_filled(egui::pos2(x + 4.0, y + 8.0), 4.0, author_colour(&v.author));
            line(&mut y, v.author.clone(), egui::FontId::proportional(13.0), TEXT, 14.0);
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() && !selected {
            self.selected = v.id.clone();
            self.asks.push(Ask::Show(v.id.clone()));
        }
        if selected {
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                let label = if v.id == CURRENT { "Name current version" } else { "Name this version" };
                if ui.add(egui::Button::new(RichText::new(label).size(13.0).color(BLUE)).frame(false)).clicked() {
                    self.naming = Some((v.id.clone(), v.name.clone(), true));
                }
            });
            ui.add_space(4.0);
        }
    }

    /// The naming dialog, when open: Google's "Name current version", a title, one field and
    /// Cancel / Save, in our dialogs' look.
    pub fn dialog(&mut self, ctx: &egui::Context) {
        let Some((id, mut text, focus)) = self.naming.take() else { return };
        let mut done = None;
        let title = if id == CURRENT { "Name current version" } else { "Name this version" };
        let modal = egui::Modal::new(egui::Id::unique("stardocs_name_version")).frame(crate::looks::dialog_frame());
        let shown = modal.show(ctx, |ui| {
            ui.set_width(400.0);
            crate::looks::title(ui, title);
            ui.add_space(16.0);
            crate::looks::field_style(ui);
            let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY).hint_text("Version name").text_color(TEXT));
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Version name"));
            if focus {
                r.request_focus();
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                done = Some(true);
            }
            ui.add_space(20.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if crate::looks::pill(ui, "Save", true).clicked() {
                    done = Some(true);
                }
                if crate::looks::pill(ui, "Cancel", false).clicked() {
                    done = Some(false);
                }
            });
        });
        if shown.should_close() {
            done.get_or_insert(false);
        }
        match done {
            Some(true) => self.asks.push(Ask::Name(id, text.trim().to_string())),
            Some(false) => {}
            None => self.naming = Some((id, text, false)),
        }
    }
}

/// A version row's height: its lines and the padding round them.
fn row_height(v: &Version) -> f32 {
    let lines = 1
        + usize::from(v.id == CURRENT)
        + usize::from(!v.name.is_empty())
        + usize::from(v.restored_from.is_some())
        + usize::from(!v.author.is_empty());
    20.0 + 18.0 + (lines - 1) as f32 * 17.0
}

/// A steady colour per person, for the dot beside their name.
fn author_colour(name: &str) -> Color32 {
    const DOTS: [Color32; 6] = [
        Color32::from_rgb(0x1A, 0x73, 0xE8),
        Color32::from_rgb(0xD9, 0x30, 0x25),
        Color32::from_rgb(0x18, 0x80, 0x38),
        Color32::from_rgb(0xE3, 0x74, 0x00),
        Color32::from_rgb(0x98, 0x1E, 0xB4),
        Color32::from_rgb(0x00, 0x7B, 0x83),
    ];
    let n = name.bytes().fold(0_usize, |h, b| h.wrapping_mul(31).wrapping_add(usize::from(b)));
    DOTS[n % DOTS.len()]
}

/// "25 Nov 2026, 14:05" in the browser's time zone (UTC on the desktop's tests).
pub fn when(secs: i64) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    #[cfg(target_arch = "wasm32")]
    let (y, m, d, h, mi) = {
        let t = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(secs as f64 * 1000.0));
        (i64::from(t.get_full_year()), i64::from(t.get_month()) + 1, i64::from(t.get_date()), i64::from(t.get_hours()), i64::from(t.get_minutes()))
    };
    #[cfg(not(target_arch = "wasm32"))]
    let (y, m, d, h, mi) = {
        // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
        let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        (yoe + era * 400 + i64::from(m <= 2), m, d, rem / 3600, rem % 3600 / 60)
    };
    let month = MONTHS.get(usize::try_from(m - 1).unwrap_or(0)).copied().unwrap_or("");
    format!("{d} {month} {y}, {h:02}:{mi:02}")
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "tests")]

    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use serde_json::json;

    fn history() -> History {
        let reply = json!({
            "current": {"saved": 1_764_072_300, "author": "ann", "name": "", "restored_from": null},
            "versions": [
                {"v": "b.docx", "saved": 1_764_068_700, "author": "boss", "name": "Final draft"},
                {"v": "a.docx", "saved": 1_763_985_600, "author": "", "name": ""},
            ],
        });
        let mut h = History::default();
        h.show();
        assert_eq!(h.take_asks(), vec![Ask::Load]);
        h.loaded(Ok(parse(&reply)));
        h
    }

    fn panel(h: History) -> Harness<'static, History> {
        let mut harness = Harness::builder().with_size(vec2(900.0, 700.0)).build_ui_state(
            |ui, h: &mut History| {
                egui::Panel::top("bar").show(ui, |ui| h.top_bar(ui));
                h.panel(ui);
                h.dialog(ui.ctx());
            },
            h,
        );
        // The dialog buttons are in WordCraft's "medium" face; the app installs it, a bare
        // window has to name one.
        let mut fonts = egui::FontDefinitions::default();
        let plain = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
        fonts.families.insert(egui::FontFamily::Name("medium".into()), plain);
        harness.ctx.set_fonts(fonts);
        harness.run();
        harness
    }

    #[test]
    fn dates_read_plainly() {
        assert_eq!(when(1_764_072_300), "25 Nov 2025, 12:05");
        assert_eq!(when(0), "1 Jan 1970, 00:00");
    }

    #[test]
    fn the_panel_lists_versions_with_who_and_names() {
        let h = panel(history());
        h.get_by_label_contains("Current version");
        h.get_by_label_contains("Final draft");
        h.get_by_label_contains("boss");
        h.get_by_label_contains("24 Nov 2025, 12:00");
        assert!(h.query_by_label("Restore this version").is_none(), "nothing to restore while the document itself is shown");
    }

    #[test]
    fn picking_a_version_shows_it_and_offers_restore_and_copy() {
        let mut h = panel(history());
        h.get_by_label_contains("Final draft").click();
        h.run();
        assert_eq!(h.state_mut().take_asks(), vec![Ask::Show("b.docx".into())]);
        h.get_by_label("Restore this version").click();
        h.run();
        assert_eq!(h.state_mut().take_asks(), vec![Ask::Restore("b.docx".into())]);
        h.get_by_label("Make a copy").click();
        h.run();
        assert_eq!(h.state_mut().take_asks(), vec![Ask::Copy("b.docx".into())]);
        h.get_by_label_contains("Current version").click();
        h.run();
        assert_eq!(h.state_mut().take_asks(), vec![Ask::Show(CURRENT.into())]);
        h.get_by_label("Back").click();
        h.run();
        assert_eq!(h.state_mut().take_asks(), vec![Ask::Close]);
        assert!(!h.state().open);
    }

    #[test]
    fn naming_the_current_version() {
        let mut h = panel(history());
        h.state_mut().name_current();
        h.run();
        h.get_by_label("Version name").type_text("Sent for review");
        h.run();
        h.get_by_label("Save").click();
        h.run();
        assert_eq!(h.state_mut().take_asks(), vec![Ask::Name(CURRENT.into(), "Sent for review".into())]);
        assert!(!h.state().naming());
    }

    #[test]
    fn an_error_is_shown_in_the_panel() {
        let mut h = History::default();
        h.show();
        h.loaded(Err(NO_SERVER.into()));
        let h = panel(h);
        h.get_by_label_contains("needs a Starbase server");
    }
}
