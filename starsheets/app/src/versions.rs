//! File › Version history, as in Google Sheets (looks review 2 item 13): the whole window turns
//! into a history view. At the top a bar with ← (back to editing), the shown version's date or
//! name and, for an earlier version, "Restore this version". In the middle that version as a
//! read-only spreadsheet. On the right, 300 px, the versions grouped by day, newest first: when,
//! "Current version", its name and who saved it (with a colour dot), and a ⋮ menu (Restore,
//! Name, Make a copy). Starbase Docs has the same layout (stardocs/app/src/versions.rs); the two
//! don't share code, as each workspace builds on its own.
//!
//! This module is the view only: what it asks for ([`Ask`]) is carried out by files.rs through
//! the server (`GET api/files/<id>/history`, `GET …/versions/<v>`, `POST …/versions/<v>/name?name=`,
//! `POST …/versions/<v>/restore`). Only editors see the history; the server refuses viewers.

use egui::{Align, Color32, Layout, Margin, RichText, Sense, Stroke, Ui, vec2};
use gridcraft_ui_egui::SheetApp;
use serde_json::Value;

use crate::{house, icons, theme};

/// The id the server gives the file as it is now.
pub const CURRENT: &str = "current";
const PANEL_WIDTH: f32 = 300.0;
const BAR_HEIGHT: f32 = 64.0;

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
    fn from_json(id: &str, v: &Value) -> Self {
        Version {
            id: id.to_string(),
            saved: v["saved"].as_i64().unwrap_or(0),
            author: v["author"].as_str().unwrap_or("").to_string(),
            name: v["name"].as_str().unwrap_or("").to_string(),
            restored_from: v["restored_from"].as_i64(),
        }
    }

    /// The row's bold line: the name when it has one, else when it was saved.
    fn title(&self) -> String {
        if self.name.is_empty() { when(self.saved) } else { self.name.clone() }
    }
}

/// The server's history reply: the current version first, then the earlier ones, newest first.
pub fn parse(v: &Value) -> Vec<Version> {
    let mut out = vec![Version::from_json(CURRENT, &v["current"])];
    for e in v["versions"].as_array().into_iter().flatten() {
        if let Some(id) = e["v"].as_str() {
            out.push(Version::from_json(id, e));
        }
    }
    out
}

/// What the view asks files.rs to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    /// Fetch the list.
    Load,
    /// Show this version (read-only), [`CURRENT`] for the file as it is now.
    Show(String),
    Restore(String),
    Copy(String),
    /// Name a version ([`CURRENT`] for the file as it is now); an empty name removes it.
    Name(String, String),
    /// Back to editing.
    Close,
}

#[derive(Default)]
pub struct History {
    pub open: bool,
    /// `None` while loading.
    pub list: Option<Vec<Version>>,
    pub error: Option<String>,
    /// The version shown ([`CURRENT`] = the file as it is now).
    pub selected: String,
    /// The shown version as a read-only spreadsheet; `None` while it loads.
    pub preview: Option<SheetApp>,
    /// The preview's sheet tabs.
    bottom: crate::bottom::Bottom,
    /// The naming dialog: which version, the name typed, and whether it takes the keyboard next.
    naming: Option<(String, String, bool)>,
    asks: Vec<Ask>,
}

impl History {
    /// File › Version history (and the clock beside Share).
    pub fn show(&mut self) {
        *self = History { open: true, selected: CURRENT.to_string(), asks: vec![Ask::Load, Ask::Show(CURRENT.into())], ..History::default() };
    }

    /// The server's answer to [`Ask::Load`].
    pub fn loaded(&mut self, list: Result<Vec<Version>, String>) {
        match list {
            Ok(l) => {
                if !l.iter().any(|v| v.id == self.selected) {
                    self.select(CURRENT);
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

    /// What the view asked for since last time.
    pub fn take_asks(&mut self) -> Vec<Ask> {
        std::mem::take(&mut self.asks)
    }

    fn select(&mut self, id: &str) {
        if self.selected != id {
            self.selected = id.to_string();
            self.preview = None;
            self.asks.push(Ask::Show(id.to_string()));
        }
    }

    pub fn close(&mut self) {
        self.open = false;
        self.preview = None;
        self.naming = None;
        self.asks.push(Ask::Close);
    }

    /// The whole history view, in place of the spreadsheet: top bar, versions panel, the
    /// shown version and the naming dialog.
    pub fn ui(&mut self, ui: &mut Ui) {
        if !self.open {
            return;
        }
        self.top_bar(ui);
        egui::Panel::right("starsheets_versions")
            .resizable(false)
            .exact_size(PANEL_WIDTH)
            .frame(egui::Frame::NONE.fill(Color32::WHITE).stroke(Stroke::new(1.0, theme::DIVIDER)).inner_margin(Margin::symmetric(0, house::L as i8)))
            .show(ui, |ui| self.panel(ui));
        match self.preview.as_mut() {
            Some(p) => {
                self.bottom.ui(p, ui);
                p.ui(ui);
            }
            None => {
                egui::CentralPanel::default().frame(egui::Frame::NONE.fill(theme::PAGE)).show(ui, |ui| {
                    ui.centered_and_justified(|ui| house::note(ui, if self.error.is_some() { "" } else { "Loading this version…" }));
                });
            }
        }
        self.dialog(ui.ctx());
        if self.naming.is_none() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.close();
        }
    }

    fn top_bar(&mut self, ui: &mut Ui) {
        let shown = self.list.as_ref().and_then(|l| l.iter().find(|v| v.id == self.selected)).map(Version::title).unwrap_or_default();
        egui::Panel::top("starsheets_versions_bar")
            .show_separator_line(false)
            .exact_size(BAR_HEIGHT)
            .frame(egui::Frame::NONE.fill(theme::PAGE).stroke(Stroke::new(1.0, theme::DIVIDER)).inner_margin(Margin::symmetric(house::L as i8, 0)))
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let back = match icons::icon("arrow_back") {
                        Some(img) => egui::Button::image(img.fit_to_exact_size(vec2(24.0, 24.0))),
                        None => egui::Button::new(RichText::new("←").size(20.0)),
                    };
                    let back = back.frame_when_inactive(false).corner_radius(20.0).min_size(vec2(40.0, 40.0));
                    if ui.add(back).on_hover_text("Back to editing").clicked() {
                        self.close();
                    }
                    ui.add_space(house::S);
                    let title = if shown.is_empty() { "Version history".to_string() } else { shown };
                    ui.label(RichText::new(title).size(house::TITLE).color(theme::TEXT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = house::S;
                        if self.selected != CURRENT && ui.add(house::primary_in("Restore this version", house::BLUE)).clicked() {
                            self.asks.push(Ask::Restore(self.selected.clone()));
                        }
                        if ui.add(house::secondary_in("Make a copy", house::BLUE)).clicked() {
                            self.asks.push(Ask::Copy(self.selected.clone()));
                        }
                    });
                });
            });
    }

    fn panel(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.add_space(house::L);
            ui.label(RichText::new("Version history").size(house::TITLE - 4.0).color(theme::TEXT));
        });
        ui.add_space(house::S);
        if let Some(e) = self.error.clone() {
            ui.horizontal_wrapped(|ui| {
                ui.add_space(house::L);
                ui.label(RichText::new(e).size(house::NOTE).color(house::ERROR));
            });
        }
        let Some(list) = self.list.clone() else {
            ui.horizontal(|ui| {
                ui.add_space(house::L);
                house::note(ui, "Loading…");
            });
            return;
        };
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut group = String::new();
            for v in &list {
                let day = day_heading(v.saved, now());
                if day != group {
                    ui.add_space(house::M);
                    ui.horizontal(|ui| {
                        ui.add_space(house::L);
                        ui.label(RichText::new(&day).size(house::NOTE).color(theme::TEXT_DIM));
                    });
                    ui.add_space(house::XS);
                    group = day;
                }
                self.row(ui, v);
            }
            if list.len() == 1 && self.error.is_none() {
                ui.add_space(house::M);
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(house::L);
                    house::note(ui, "Earlier versions appear here once the spreadsheet has been saved again.");
                });
            }
        });
    }

    /// One version: bold when, then "Current version", its name or restore note, and who saved it.
    fn row(&mut self, ui: &mut Ui, v: &Version) {
        let selected = v.id == self.selected;
        let mut lines: Vec<(String, bool)> = Vec::new();
        if v.id == CURRENT {
            lines.push(("Current version".into(), true));
        }
        if !v.name.is_empty() {
            lines.push((when(v.saved), false));
        }
        if let Some(from) = v.restored_from {
            lines.push((format!("Restored from {}", when(from)), false));
        }
        let height = 12.0 + 20.0 + 18.0 * (lines.len() as f32 + f32::from(u8::from(!v.author.is_empty())));
        let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
        let p = ui.painter();
        let fill = if selected {
            theme::ON
        } else if r.hovered() {
            theme::MENU_HOVER
        } else {
            Color32::TRANSPARENT
        };
        p.rect_filled(rect, 0.0, fill);
        let left = rect.left() + house::L + 8.0;
        let mut y = rect.top() + 6.0;
        p.text(egui::pos2(left, y), egui::Align2::LEFT_TOP, v.title(), gridcraft_ui_egui::theme::ui_bold(house::BODY), theme::TEXT);
        y += 20.0;
        for (text, italic) in &lines {
            let font = egui::FontId::proportional(house::NOTE + 1.0);
            let colour = if *italic { theme::TEXT_DIM } else { theme::TEXT };
            p.text(egui::pos2(left, y), egui::Align2::LEFT_TOP, text, font, colour);
            y += 18.0;
        }
        if !v.author.is_empty() {
            p.circle_filled(egui::pos2(left + 4.0, y + 8.0), 4.0, person_colour(&v.author));
            p.text(egui::pos2(left + 14.0, y), egui::Align2::LEFT_TOP, &v.author, egui::FontId::proportional(house::NOTE + 1.0), theme::TEXT);
        }
        if selected {
            p.vline(rect.left() + 2.0, rect.y_range(), Stroke::new(3.0, house::BLUE));
        }
        // ⋮ at the right of the selected or hovered row.
        let mut on_menu = false;
        if selected || r.hovered() {
            let at = egui::Rect::from_min_size(egui::pos2(rect.right() - 44.0, rect.top() + 4.0), vec2(32.0, 32.0));
            let more = match icons::icon("more_vert") {
                Some(img) => egui::Button::image(img.fit_to_exact_size(vec2(20.0, 20.0))),
                None => egui::Button::new("⋮"),
            }
            .frame_when_inactive(false)
            .corner_radius(16.0);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(at));
            let (resp, _) = egui::containers::menu::MenuButton::from_button(more).ui(&mut child, |ui| {
                house::menu(ui, 240.0);
                if v.id != CURRENT && house::menu_row(ui, Some("history"), "Restore this version", "", true) {
                    self.asks.push(Ask::Restore(v.id.clone()));
                    ui.close();
                }
                let label = if v.id == CURRENT { "Name current version" } else { "Name this version" };
                if house::menu_row(ui, Some("edit"), label, "", true) {
                    self.naming = Some((v.id.clone(), v.name.clone(), true));
                    ui.close();
                }
                if house::menu_row(ui, Some("content_copy"), "Make a copy", "", true) {
                    self.asks.push(Ask::Copy(v.id.clone()));
                    ui.close();
                }
            });
            on_menu = resp.hovered() || resp.clicked();
            // The menu stays with the row it was opened on.
            if resp.clicked() {
                self.select(&v.id);
            }
            resp.on_hover_text("More actions");
        }
        let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
        r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, v.title()));
        if r.clicked() && !on_menu {
            self.select(&v.id);
        }
    }

    /// The naming dialog, when open (house style, as Sheets' "Name this version").
    fn dialog(&mut self, ctx: &egui::Context) {
        let Some((id, mut text, focus)) = self.naming.take() else { return };
        let mut close = false;
        let title = if id == CURRENT { "Name current version" } else { "Name this version" };
        let done = house::dialog(ctx, "starsheets_name_version", 400.0, title, Some(&mut close), |ui| {
            let r = ui.add(house::field(&mut text, house::BODY).hint_text("Version name").desired_width(f32::INFINITY));
            if focus {
                r.request_focus();
            }
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            match house::actions_in(ui, &["Cancel", "Save"], house::BLUE) {
                Some(1) => Some(true),
                Some(_) => Some(false),
                None if enter => Some(true),
                None => None,
            }
        });
        let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        match (done, close || escape) {
            (Some(true), _) => self.asks.push(Ask::Name(id, text.trim().to_string())),
            (Some(false), _) | (None, true) => {}
            (None, false) => self.naming = Some((id, text, false)),
        }
    }
}

/// A person's colour dot: one of a few clear colours, the same for a name every time.
fn person_colour(name: &str) -> Color32 {
    const COLOURS: [Color32; 8] = [
        Color32::from_rgb(0x1A, 0x73, 0xE8),
        Color32::from_rgb(0xD9, 0x30, 0x25),
        Color32::from_rgb(0x18, 0x80, 0x38),
        Color32::from_rgb(0xE3, 0x74, 0x00),
        Color32::from_rgb(0x9C, 0x27, 0xB0),
        Color32::from_rgb(0x00, 0x89, 0x7B),
        Color32::from_rgb(0xC2, 0x18, 0x5B),
        Color32::from_rgb(0x5F, 0x63, 0x68),
    ];
    let h = name.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
    COLOURS[h as usize % COLOURS.len()]
}

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// (year, month 1-12, day, hour, minute) in the browser's time zone (UTC elsewhere).
fn parts(secs: i64) -> (i64, i64, i64, i64, i64) {
    #[cfg(target_arch = "wasm32")]
    {
        let t = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(secs as f64 * 1000.0));
        (i64::from(t.get_full_year()), i64::from(t.get_month()) + 1, i64::from(t.get_date()), i64::from(t.get_hours()), i64::from(t.get_minutes()))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
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
    }
}

fn now() -> i64 {
    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Date::now() / 1000.0) as i64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
    }
}

/// "25 November, 14:05" (the year too when it isn't this one), as Sheets labels a version.
pub fn when(secs: i64) -> String {
    let (y, m, d, h, mi) = parts(secs);
    let month = MONTHS.get(usize::try_from(m - 1).unwrap_or(0)).copied().unwrap_or("");
    if y == parts(now()).0 { format!("{d} {month}, {h:02}:{mi:02}") } else { format!("{d} {month} {y}, {h:02}:{mi:02}") }
}

/// The heading a version is grouped under: "Today", "Yesterday", else its month ("November 2026").
fn day_heading(secs: i64, now: i64) -> String {
    let day = |s: i64| {
        let (y, m, d, ..) = parts(s);
        (y, m, d)
    };
    if day(secs) == day(now) {
        "Today".into()
    } else if day(secs) == day(now - 86_400) {
        "Yesterday".into()
    } else {
        let (y, m, _) = day(secs);
        format!("{} {y}", MONTHS.get(usize::try_from(m - 1).unwrap_or(0)).copied().unwrap_or(""))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_history_reply_lists_the_current_version_first() {
        let reply = json!({
            "current": {"saved": 1_764_072_300, "author": "ann", "name": "", "restored_from": null},
            "versions": [
                {"v": "b.xlsx", "saved": 1_764_068_700, "author": "boss", "name": "Final"},
                {"v": "a.xlsx", "saved": 1_763_985_600, "author": "", "name": ""},
            ],
        });
        let list = parse(&reply);
        assert_eq!(list.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), [CURRENT, "b.xlsx", "a.xlsx"]);
        assert_eq!(list[1].title(), "Final");
        assert_eq!(list[1].author, "boss");
    }

    #[test]
    fn versions_group_by_day_then_month() {
        let now = 1_764_072_300; // 25 Nov 2025, 12:05 UTC
        assert_eq!(day_heading(now - 3600, now), "Today");
        assert_eq!(day_heading(now - 86_400, now), "Yesterday");
        assert_eq!(day_heading(now - 20 * 86_400, now), "November 2025");
        assert_eq!(day_heading(0, now), "January 1970");
        assert_eq!(when(0), "1 January 1970, 00:00");
    }

    #[test]
    fn picking_a_version_asks_to_show_it_and_closing_asks_to_go_back() {
        let mut h = History::default();
        h.show();
        assert_eq!(h.take_asks(), vec![Ask::Load, Ask::Show(CURRENT.into())]);
        h.select("b.xlsx");
        h.select("b.xlsx");
        assert_eq!(h.take_asks(), vec![Ask::Show("b.xlsx".into())]);
        h.loaded(Ok(vec![Version { id: CURRENT.into(), ..Version::default() }]));
        assert_eq!(h.selected, CURRENT, "a version that's gone falls back to the current one");
        h.close();
        assert!(!h.open);
        assert_eq!(h.take_asks(), vec![Ask::Show(CURRENT.into()), Ask::Close]);
    }

    #[test]
    fn a_person_keeps_one_colour() {
        assert_eq!(person_colour("ada"), person_colour("ada"));
    }
}
