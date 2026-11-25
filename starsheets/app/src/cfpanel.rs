//! Conditional formatting as a side panel, the way Google Sheets does it (docs/sheets-cf.md):
//! a list of rule cards, and an editor with two tabs, "Single colour" (a condition plus a style)
//! and "Colour scale" (min / mid / max points, each with a colour). Rules are stored and drawn
//! by GridCraft (`home.conditionalFormat`, `home.manageRules`; colour-scale points: patch 0017).

use egui::{Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, vec2};
use gridcraft_engine::model::{CfOperator, CfRule, CfValueKind, Color, CondFormat, Underline};
use gridcraft_ui_egui::SheetApp;
use serde_json::{Value as Json, json};

use crate::theme;

const GREEN: Color32 = crate::house::GREEN;
const PANEL_W: f32 = 320.0;

/// Google Sheets' conditions, in its order: label, GridCraft rule type, operator, value boxes.
const CONDITIONS: &[(&str, &str, &str, u8)] = &[
    ("Is empty", "blanks", "", 0),
    ("Is not empty", "noBlanks", "", 0),
    ("Text contains", "containsText", "", 1),
    ("Text does not contain", "notContainsText", "", 1),
    ("Text starts with", "beginsWith", "", 1),
    ("Text ends with", "endsWith", "", 1),
    ("Text is exactly", "cellIs", "equal", 1),
    ("Date is", "date", "equal", 1),
    ("Date is before", "date", "less", 1),
    ("Date is after", "date", "greater", 1),
    ("Greater than", "cellIs", "greater", 1),
    ("Greater than or equal to", "cellIs", "greaterOrEqual", 1),
    ("Less than", "cellIs", "less", 1),
    ("Less than or equal to", "cellIs", "lessOrEqual", 1),
    ("Is equal to", "cellIs", "equal", 1),
    ("Is not equal to", "cellIs", "notEqual", 1),
    ("Is between", "cellIs", "between", 2),
    ("Is not between", "cellIs", "notBetween", 2),
    ("Custom formula is", "expression", "", 1),
];

/// Sheets' preset styles (the menu under "Formatting style"): (text colour, fill). Top row:
/// green, yellow, red fills; bottom row: the same colours as text only.
const STYLE_PRESETS: &[(Option<u32>, Option<u32>)] =
    &[(None, Some(0xB7E1CD)), (None, Some(0xFCE8B2)), (None, Some(0xF4C7C3)), (Some(0x0B8043), None), (Some(0xF09300), None), (Some(0xC5221F), None)];

/// Colour-scale presets (min, max; Sheets' first row is two-colour, the rest add a midpoint).
const SCALE_PRESETS: &[&[u32]] = &[
    &[0xFFFFFF, 0x57BB8A],
    &[0x57BB8A, 0xFFFFFF],
    &[0xFFFFFF, 0xE67C73],
    &[0xE67C73, 0xFFFFFF],
    &[0xFFFFFF, 0x7BAAF7],
    &[0xFFFFFF, 0xFFD666],
    &[0xE67C73, 0xFFD666, 0x57BB8A],
    &[0x57BB8A, 0xFFD666, 0xE67C73],
    &[0xFFFFFF, 0xFFD666, 0xE67C73],
];

/// Point types: label, GridCraft value type ("" = no midpoint).
const POINT_KINDS: &[(&str, &str)] =
    &[("None", ""), ("Min value", "min"), ("Max value", "max"), ("Number", "num"), ("Percent", "percent"), ("Percentile", "percentile")];

#[derive(Clone)]
struct Point {
    kind: usize,
    value: String,
    colour: u32,
}

#[derive(Clone)]
struct Draft {
    /// The rule being edited (its place in the sheet's list), or None for a new one.
    index: Option<usize>,
    scale: bool,
    range: String,
    condition: usize,
    /// For the date conditions: which of Sheets' choices (0 = exact date; see date_choices).
    when: usize,
    value1: String,
    value2: String,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    text: Option<u32>,
    fill: Option<u32>,
    /// Minpoint, midpoint, maxpoint.
    points: [Point; 3],
}

impl Draft {
    fn new(range: String) -> Draft {
        Draft {
            index: None,
            scale: false,
            range,
            condition: 1,
            when: 0,
            value1: String::new(),
            value2: String::new(),
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            text: None,
            fill: STYLE_PRESETS[0].1,
            points: [
                Point { kind: 1, value: String::new(), colour: 0xFFFFFF },
                Point { kind: 0, value: String::new(), colour: 0xFFD666 },
                Point { kind: 2, value: String::new(), colour: 0x57BB8A },
            ],
        }
    }
}

#[derive(Default)]
pub struct CfPanel {
    pub open: bool,
    /// Opened by this frame's intercept (panels.rs).
    pub just_opened: bool,
    draft: Option<Draft>,
    error: Option<String>,
    /// Live preview while editing, as in Sheets: the draft is on the sheet for real. Each change
    /// edits that one rule in place (`replace`), and Cancel puts the edited rule back exactly or
    /// deletes a new one. No undo: with live editing undo is the server's, and undos mixed with
    /// edits in one frame came back out of step (rules vanished and came back, to-do 2 row 14).
    /// The plan last applied (applying the same again is skipped).
    key: Option<String>,
    /// The preview rule on the sheet: where it was put and what it was, so it is found again by
    /// what it is even if other rules moved.
    shown: Option<(usize, CondFormat)>,
    /// The rule being edited as it was before the panel touched it (put back on Cancel).
    original: Option<CondFormat>,
}

impl CfPanel {
    /// Opens the panel (from Format › Conditional formatting or the right-click menu): the rule
    /// list when the sheet has rules, else a new rule for the selection, as in Sheets.
    pub fn open(&mut self, app: &SheetApp) {
        self.open = true;
        self.error = None;
        let h = crate::hostdata::read(app);
        let has_rules = app
            .session
            .active()
            .and_then(|d| d.wb.active())
            .is_some_and(|sh| sh.cond_formats.iter().any(|cf| !(crate::dvpanel::is_item_colour(sh, cf, &h) || crate::altcolours::is_ours(cf))));
        self.draft = if has_rules { None } else { Some(Draft::new(selection_text(app))) };
    }

    /// Closes the panel (another side panel opened), undoing any preview.
    pub fn close(&mut self, app: &mut SheetApp) {
        self.revert(app);
        self.draft = None;
        self.open = false;
    }

    /// Takes over GridCraft's own conditional-formatting dialogs (they open from its commands).
    pub fn intercept(&mut self, app: &mut SheetApp) {
        let ours = app.dialog.as_ref().is_some_and(|d| {
            d.command == Some("home.conditionalFormat") || matches!(d.name.as_str(), "manageRules" | "cfQuick" | "newFormattingRule")
        });
        if ours {
            app.dialog = None;
            self.open(app);
        }
    }

    pub fn ui(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        if !self.open {
            return;
        }
        egui::Panel::right("starsheets_cf")
            .exact_size(PANEL_W + 32.0 + crate::house::SCROLL_ROOM)
            .resizable(false)
            .frame(crate::house::panel_frame())
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Conditional format rules").size(16.0).color(theme::TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new(RichText::new("✕").size(16.0)).frame(false)).on_hover_text("Close").clicked() {
                            self.revert(app);
                            self.draft = None;
                            self.open = false;
                        }
                    });
                });
                ui.add_space(8.0);
                egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(crate::house::panel_scroll_height(ui)).show(ui, |ui| {
                    crate::house::form_style(ui);
                    ui.set_width(PANEL_W);
                    if let Some(mut d) = self.draft.take() {
                        match self.editor(app, ui, &mut d) {
                            Some(true) => {
                                if self.save(app, &d) {
                                    self.draft = None;
                                } else {
                                    self.draft = Some(d);
                                }
                            }
                            Some(false) => {
                                self.revert(app);
                                self.error = None;
                                let h = crate::hostdata::read(app);
                                let none_left = app.session.active().and_then(|d| d.wb.active()).is_none_or(|sh| {
                                    sh.cond_formats.iter().all(|cf| crate::dvpanel::is_item_colour(sh, cf, &h) || crate::altcolours::is_ours(cf))
                                });
                                if none_left {
                                    self.open = false;
                                }
                            }
                            None => {
                                self.update_preview(app, &d);
                                self.draft = Some(d);
                            }
                        }
                    } else {
                        self.list(app, ui);
                    }
                });
            });
    }

    /// The rule cards: a swatch, what the rule does and its range; click to edit, bin to delete.
    fn list(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        let Some((rules, theme_colours)) = app.session.active().and_then(|d| d.wb.active().map(|sh| (sh.cond_formats.clone(), d.wb.theme.clone())))
        else {
            return;
        };
        let mut delete = None;
        let h = crate::hostdata::read(app);
        let hidden: Vec<usize> = app
            .session
            .active()
            .and_then(|d| d.wb.active())
            .map(|sh| {
                sh.cond_formats
                    .iter()
                    .enumerate()
                    .filter(|(_, cf)| crate::dvpanel::is_item_colour(sh, cf, &h) || crate::altcolours::is_ours(cf))
                    .map(|(i, _)| i)
                    .collect()
            })
            .unwrap_or_default();
        for (i, cf) in rules.iter().enumerate() {
            // A drop-down item's colour is kept as a rule of this kind, but it belongs to the
            // data validation panel (as in Sheets), so it isn't listed here.
            if hidden.contains(&i) {
                continue;
            }
            let ranges = cf.ranges.iter().map(|r| r.a1()).collect::<Vec<_>>().join(",");
            let (r, resp) = ui.allocate_exact_size(vec2(PANEL_W, 60.0), Sense::click());
            let p = ui.painter();
            if resp.hovered() {
                p.rect_filled(r, 0.0, theme::MENU_HOVER);
            }
            let sw = egui::Rect::from_min_size(r.min + vec2(4.0, 8.0), vec2(44.0, 44.0));
            paint_swatch(p, sw, &cf.rule, &theme_colours);
            p.text(
                sw.right_center() + vec2(12.0, -9.0),
                egui::Align2::LEFT_CENTER,
                describe(&cf.rule),
                egui::FontId::proportional(13.5),
                theme::TEXT,
            );
            p.text(sw.right_center() + vec2(12.0, 10.0), egui::Align2::LEFT_CENTER, ranges, egui::FontId::proportional(12.0), theme::TEXT_DIM);
            // The bin always exists (so the pointer moving onto it doesn't make it vanish) and
            // shows while the card or the bin is hovered.
            let bin = egui::Rect::from_center_size(egui::pos2(r.right() - 20.0, r.center().y), vec2(28.0, 28.0));
            let b = ui.interact(bin, ui.scope_id().with(("cf_bin", i)), Sense::click());
            if (resp.hovered() || b.hovered())
                && let Some(img) = crate::icons::icon("delete")
            {
                img.paint_at(ui, bin.shrink(5.0));
            }
            if b.on_hover_text("Remove rule").clicked() {
                delete = Some(i);
                continue;
            }
            if resp.clicked() {
                let mut d = draft_from(&cf.rule, &theme_colours).unwrap_or_else(|| Draft::new(String::new()));
                d.range = cf.ranges.iter().map(|r| r.a1()).collect::<Vec<_>>().join(",");
                d.index = Some(i);
                read_date_choice(&mut d);
                self.draft = Some(d);
                self.original = Some(cf.clone());
                self.shown = Some((i, cf.clone()));
                self.key = None;
            }
            ui.painter().hline(r.x_range(), r.bottom(), Stroke::new(1.0, theme::DIVIDER));
        }
        if let Some(i) = delete {
            app.run_or_alert("home.manageRules", json!({ "delete": i }));
        }
        ui.add_space(12.0);
        if ui.add(egui::Button::new(RichText::new("+  Add another rule").size(14.0).color(GREEN)).frame(false)).clicked() {
            self.draft = Some(Draft::new(selection_text(app)));
            self.original = None;
            self.shown = None;
            self.key = None;
        }
    }

    /// The rule editor. Returns Some(true) for Done, Some(false) for Cancel.
    fn editor(&mut self, _app: &mut SheetApp, ui: &mut Ui, d: &mut Draft) -> Option<bool> {
        // Tabs.
        ui.columns(2, |cols| {
            for (i, (label, scale)) in [("Single colour", false), ("Colour scale", true)].into_iter().enumerate() {
                let on = d.scale == scale;
                let col = &mut cols[i];
                let r = col.add(
                    egui::Button::new(RichText::new(label).size(14.0).color(if on { GREEN } else { theme::TEXT }))
                        .frame(false)
                        .min_size(vec2(col.available_width(), 32.0)),
                );
                if on {
                    col.painter().hline(r.rect.x_range().shrink(20.0), r.rect.bottom(), Stroke::new(3.0, GREEN));
                }
                if r.clicked() {
                    d.scale = scale;
                }
            }
        });
        ui.add_space(10.0);
        ui.label(RichText::new("Apply to range").size(14.0).strong());
        ui.add(egui::TextEdit::singleline(&mut d.range).desired_width(PANEL_W).margin(vec2(8.0, 8.0)));
        ui.add_space(12.0);
        ui.label(RichText::new("Format rules").size(14.0).strong());
        ui.add_space(4.0);
        if d.scale {
            scale_editor(ui, d);
        } else {
            single_editor(ui, d);
        }
        if let Some(e) = &self.error {
            ui.add_space(6.0);
            ui.label(RichText::new(e).color(Color32::from_rgb(0xC5, 0x39, 0x29)));
        }
        ui.add_space(14.0);
        let mut out = None;
        ui.horizontal(|ui| {
            ui.add_space(PANEL_W - 200.0);
            if ui.add(crate::house::secondary("Cancel")).clicked() {
                out = Some(false);
            }
            if ui.add(crate::house::primary("Done")).clicked() {
                out = Some(true);
            }
        });
        out
    }

    /// Where the preview rule is now (found by what it is when other rules moved), if still there.
    fn locate(app: &SheetApp, at: &(usize, CondFormat)) -> Option<usize> {
        let sh = app.session.active()?.wb.active()?;
        if sh.cond_formats.get(at.0) == Some(&at.1) {
            return Some(at.0);
        }
        sh.cond_formats.iter().position(|cf| *cf == at.1)
    }

    /// Takes the preview off: the edited rule goes back exactly as it was, a new one goes.
    fn revert(&mut self, app: &mut SheetApp) {
        self.key = None;
        let (Some(at), original) = (self.shown.take(), self.original.take()) else { return };
        let Some(i) = Self::locate(app, &at) else { return };
        match original {
            Some(cf) if cf != at.1 => {
                let _ = app.run("home.manageRules", json!({ "set": i, "cf": serde_json::to_value(&cf).unwrap_or_default() }));
            }
            Some(_) => {}
            None => {
                let _ = app.run("home.manageRules", json!({ "delete": i }));
            }
        }
    }

    /// Puts the draft on the sheet: in place of the preview (or the edited rule), or as a new
    /// rule (GridCraft adds new rules first). Returns the engine's complaint, if any.
    fn apply(&mut self, app: &mut SheetApp, d: &Draft, rule: Json) -> Result<(), String> {
        if let Some(other) =
            crate::refs::other_sheet(&d.range, &app.session.active().and_then(|x| x.wb.active()).map(|sh| sh.name.clone()).unwrap_or_default())
        {
            return Err(format!("The range must be on this sheet, not \"{other}\"."));
        }
        let mut p = json!({ "range": crate::refs::plain(&d.range), "rule": rule });
        let at = self.shown.as_ref().and_then(|at| Self::locate(app, at));
        if let Some(i) = at {
            p["replace"] = json!(i);
        }
        app.run("home.conditionalFormat", p).map_err(|e| crate::messages::plain_error(&gridcraft_ui_egui::clean_error(&e)))?;
        let i = at.unwrap_or(0);
        self.shown = app.session.active().and_then(|x| x.wb.active()).and_then(|sh| sh.cond_formats.get(i).cloned()).map(|cf| (i, cf));
        Ok(())
    }

    /// Applies the draft as it stands, when it changed. A draft that isn't complete yet (e.g. no
    /// value typed) shows what was there before the panel.
    fn update_preview(&mut self, app: &mut SheetApp, d: &Draft) {
        let rule = rule_json(d);
        let key = format!("{}|{}", d.range.trim(), rule.as_ref().map(ToString::to_string).unwrap_or_default());
        if self.key.as_ref() == Some(&key) {
            return;
        }
        match rule {
            Ok(rule) => {
                let _ = self.apply(app, d, rule);
            }
            Err(_) => {
                let original = self.original.clone();
                self.revert(app);
                // Still editing: the restored rule is where the next preview goes.
                if let (Some(cf), Some(i)) = (original, d.index) {
                    self.shown = Some((i, cf.clone()));
                    self.original = Some(cf);
                }
            }
        }
        self.key = Some(key);
    }

    /// Done: the draft stays on the sheet. On failure the panel stays open with the reason.
    fn save(&mut self, app: &mut SheetApp, d: &Draft) -> bool {
        let rule = match rule_json(d) {
            Ok(r) => r,
            Err(e) => {
                self.error = Some(e);
                return false;
            }
        };
        match self.apply(app, d, rule) {
            Ok(()) => {
                self.error = None;
                self.shown = None;
                self.original = None;
                self.key = None;
                true
            }
            Err(e) => {
                self.error = Some(e);
                false
            }
        }
    }
}

fn selection_text(app: &SheetApp) -> String {
    app.session.active().map(|d| d.selection.ranges.iter().map(|r| r.a1()).collect::<Vec<_>>().join(",")).unwrap_or_default()
}

pub(crate) fn c32(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub(crate) fn rgb_of(c: &Color, theme_colours: &gridcraft_engine::model::Theme) -> Option<u32> {
    c.resolve(theme_colours).map(|[r, g, b]| (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b))
}

pub(crate) fn hex(rgb: u32) -> String {
    format!("#{rgb:06X}")
}

/// A colour button that opens the palette.
pub(crate) fn colour_pick(ui: &mut Ui, id: &str, colour: Option<u32>, icon: &str, tip: &str) -> Option<Option<u32>> {
    let mut picked = None;
    let r = ui
        .menu_button(crate::icons::icon(icon).map_or_else(|| egui::Atom::from(tip.to_string()), egui::Atom::from), |ui| {
            match crate::colourpicker::palette(ui, colour, false, false) {
                Some(crate::colourpicker::Choice::Reset) => picked = Some(None),
                Some(crate::colourpicker::Choice::Colour(rgb)) => picked = Some(Some(rgb)),
                _ => {}
            }
        })
        .response;
    // The chosen colour as a bar under the icon, like Sheets.
    let bar = egui::Rect::from_min_size(egui::pos2(r.rect.left() + 4.0, r.rect.bottom() - 4.0), vec2(r.rect.width() - 8.0, 3.0));
    ui.painter().rect_filled(bar, 0.0, colour.map_or(Color32::TRANSPARENT, c32));
    let _ = id;
    r.on_hover_text(tip);
    picked
}

fn single_editor(ui: &mut Ui, d: &mut Draft) {
    ui.label(RichText::new("Format cells if…").size(12.0).color(theme::TEXT_DIM));
    // The whole list at once, with Sheets' dividers.
    let labels: Vec<&str> = CONDITIONS.iter().map(|c| c.0).collect();
    crate::house::select(ui, "cf_condition", PANEL_W, &mut d.condition, &labels, &[6, 9, 17]);
    let (_, kind, _, boxes) = CONDITIONS[d.condition];
    let hint = match kind {
        "date" => "Date, e.g. 25/12/2026",
        "expression" => "Formula, e.g. =A1>10",
        _ => "Value or formula",
    };
    let (_, _, op, _) = CONDITIONS[d.condition];
    if kind == "date" {
        let choices = date_choices(op);
        d.when = d.when.min(choices.len() - 1);
        let labels: Vec<&str> = choices.iter().map(|c| c.0).collect();
        crate::house::select(ui, "cf_when", PANEL_W, &mut d.when, &labels, &[]);
    }
    if boxes >= 1 && !(kind == "date" && d.when > 0) {
        ui.add(egui::TextEdit::singleline(&mut d.value1).hint_text(hint).desired_width(PANEL_W).margin(vec2(8.0, 8.0)));
    }
    if boxes >= 2 {
        ui.add(egui::TextEdit::singleline(&mut d.value2).hint_text("and").desired_width(PANEL_W).margin(vec2(8.0, 8.0)));
    }
    ui.add_space(10.0);
    ui.label(RichText::new("Formatting style").size(12.0).color(theme::TEXT_DIM));
    // Preview, as in Sheets: "Default" in the chosen style.
    // The style preview; click it for Sheets' preset styles.
    let (r, preview) = ui.allocate_exact_size(vec2(PANEL_W, 36.0), Sense::click());
    ui.painter().rect_filled(r, CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, d.fill.map_or(Color32::WHITE, c32));
    let colour = d.text.map_or(theme::TEXT, c32);
    let style_text = egui::text::LayoutJob::simple_format(
        "Default".into(),
        egui::TextFormat {
            font_id: egui::FontId::proportional(14.0),
            color: colour,
            italics: d.italic,
            underline: if d.underline { Stroke::new(1.0, colour) } else { Stroke::NONE },
            strikethrough: if d.strike { Stroke::new(1.0, colour) } else { Stroke::NONE },
            ..Default::default()
        },
    );
    let galley = ui.painter().layout_job(style_text);
    let at = egui::pos2(r.left() + 8.0, r.center().y - galley.size().y / 2.0);
    // Bold: drawn twice, half a pixel apart (the UI font has no bold face).
    if d.bold {
        ui.painter().galley(at + vec2(0.6, 0.0), galley.clone(), colour);
    }
    ui.painter().galley(at, galley, colour);
    Frame::NONE.stroke(Stroke::new(1.0, theme::DIVIDER)).inner_margin(Margin::symmetric(6, 4)).show(ui, |ui| {
        ui.set_width(PANEL_W - 14.0);
        ui.horizontal(|ui| {
            for (flag, label, tip) in [
                (&mut d.bold, "B", "Bold"),
                (&mut d.italic, "I", "Italic"),
                (&mut d.underline, "U", "Underline"),
                (&mut d.strike, "S", "Strikethrough"),
            ] {
                let mut t = RichText::new(label).size(15.0);
                t = match label {
                    "B" => t.strong(),
                    "I" => t.italics(),
                    "U" => t.underline(),
                    _ => t.strikethrough(),
                };
                if ui.add(egui::Button::new(t).selected(*flag).frame_when_inactive(false).min_size(vec2(28.0, 28.0))).on_hover_text(tip).clicked() {
                    *flag = !*flag;
                }
            }
            ui.add_space(4.0);
            if let Some(c) = colour_pick(ui, "cf_text", d.text, "format_color_text", "Text colour") {
                d.text = c;
            }
            if let Some(c) = colour_pick(ui, "cf_fill", d.fill, "format_color_fill", "Fill colour") {
                d.fill = c;
            }
        });
    });
    ui.add_space(6.0);
    let menu_id = ui.scope_id().with("cf_style_presets");
    if preview.on_hover_text("Preset styles").clicked() {
        egui::Popup::toggle_id(ui.ctx(), menu_id);
    }
    egui::Popup::new(menu_id, ui.ctx().clone(), egui::PopupAnchor::Position(r.left_bottom() + vec2(0.0, 4.0)), ui.layer_id())
        .open_memory(None)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            egui::Grid::new("cf_style_grid").spacing(vec2(12.0, 12.0)).show(ui, |ui| {
                for (i, (text, fill)) in STYLE_PRESETS.iter().enumerate() {
                    let (cell, resp) = ui.allocate_exact_size(vec2(70.0, 32.0), Sense::click());
                    let p = ui.painter();
                    p.rect_filled(cell, 2.0, fill.map_or(Color32::WHITE, c32));
                    p.rect_stroke(cell, 2.0, Stroke::new(1.0, fill.map_or(theme::DIVIDER, c32)), egui::StrokeKind::Inside);
                    let current = d.text == *text && d.fill == *fill;
                    let label = if current { "✔123" } else { "123" };
                    p.text(cell.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(13.0), text.map_or(theme::TEXT, c32));
                    if resp.hovered() {
                        p.rect_stroke(cell, 2.0, Stroke::new(1.5, theme::BLUE), egui::StrokeKind::Outside);
                    }
                    if resp.clicked() {
                        d.text = *text;
                        d.fill = *fill;
                        d.bold = false;
                        d.italic = false;
                        d.underline = false;
                        d.strike = false;
                        egui::Popup::close_id(ui.ctx(), menu_id);
                    }
                    if i % 3 == 2 {
                        ui.end_row();
                    }
                }
            });
            ui.separator();
            // Custom format: the bar under the preview (B, I, U, S, colours) does the rest.
            if ui.add(egui::Button::new("Custom format").min_size(vec2(234.0, 28.0))).on_hover_text("Use the buttons under the preview").clicked() {
                egui::Popup::close_id(ui.ctx(), menu_id);
            }
        });
}

fn scale_editor(ui: &mut Ui, d: &mut Draft) {
    ui.label(RichText::new("Preview").size(12.0).color(theme::TEXT_DIM));
    let (r, resp) = ui.allocate_exact_size(vec2(PANEL_W, 36.0), Sense::click());
    paint_gradient(ui.painter(), r, &scale_colours(d));
    ui.painter().text(r.left_center() + vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, "Default", egui::FontId::proportional(14.0), theme::TEXT);
    let presets_id = ui.scope_id().with("cf_presets");
    if resp.on_hover_text("Choose a preset").clicked() {
        egui::Popup::toggle_id(ui.ctx(), presets_id);
    }
    egui::Popup::new(presets_id, ui.ctx().clone(), egui::PopupAnchor::Position(r.left_bottom()), ui.layer_id()).open_memory(None).show(|ui| {
        for p in SCALE_PRESETS {
            let (pr, presp) = ui.allocate_exact_size(vec2(220.0, 22.0), Sense::click());
            let cols: Vec<Color32> = p.iter().map(|c| c32(*c)).collect();
            paint_gradient(ui.painter(), pr, &cols);
            if presp.clicked() {
                d.points[0].colour = p[0];
                d.points[2].colour = p[p.len() - 1];
                if p.len() == 3 {
                    d.points[1].kind = 5;
                    d.points[1].value = "50".into();
                    d.points[1].colour = p[1];
                } else {
                    d.points[1].kind = 0;
                }
                egui::Popup::close_id(ui.ctx(), presets_id);
            }
            ui.add_space(4.0);
        }
    });
    for (i, title) in ["Minpoint", "Midpoint", "Maxpoint"].into_iter().enumerate() {
        ui.add_space(8.0);
        ui.label(RichText::new(title).size(12.0).color(theme::TEXT_DIM));
        let p = &mut d.points[i];
        ui.horizontal(|ui| {
            // Min only offers min / number / percent / percentile; mid adds None; max offers max.
            let kinds: Vec<usize> = match i {
                0 => vec![1, 3, 4, 5],
                1 => vec![0, 3, 4, 5],
                _ => vec![2, 3, 4, 5],
            };
            let labels: Vec<&str> = kinds.iter().map(|k| POINT_KINDS[*k].0).collect();
            let mut at = kinds.iter().position(|k| *k == p.kind).unwrap_or(0);
            if crate::house::select(ui, &format!("cf_point_{i}"), 120.0, &mut at, &labels, &[]) {
                p.kind = kinds[at];
            }
            let takes_value = matches!(POINT_KINDS[p.kind].1, "num" | "percent" | "percentile");
            ui.add_enabled(takes_value, egui::TextEdit::singleline(&mut p.value).hint_text("Value").desired_width(100.0));
            if p.kind != 0
                && let Some(Some(c)) = colour_pick(ui, "cf_point_colour", Some(p.colour), "format_color_fill", "Colour")
            {
                p.colour = c;
            }
        });
    }
}

fn scale_colours(d: &Draft) -> Vec<Color32> {
    let mut v = vec![c32(d.points[0].colour)];
    if d.points[1].kind != 0 {
        v.push(c32(d.points[1].colour));
    }
    v.push(c32(d.points[2].colour));
    v
}

fn paint_gradient(p: &egui::Painter, r: egui::Rect, cols: &[Color32]) {
    let n = 24;
    for k in 0..n {
        let t = k as f32 / (n - 1) as f32;
        let seg = t * (cols.len() - 1) as f32;
        let i = (seg.floor() as usize).min(cols.len() - 2);
        let c = lerp(cols[i], cols[i + 1], seg - i as f32);
        let x0 = r.left() + r.width() * k as f32 / n as f32;
        let x1 = r.left() + r.width() * (k + 1) as f32 / n as f32 + 0.5;
        p.rect_filled(egui::Rect::from_x_y_ranges(x0..=x1, r.y_range()), 0.0, c);
    }
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// Days since 1899-12-30 (the spreadsheet date number) for "dd/mm/yyyy".
/// Sheets' choices under "Date is" (op equal) or "Date is before / after": label and the formula
/// for the date (k = days from today) ; index 0 is a typed exact date.
fn date_choices(op: &str) -> &'static [(&'static str, &'static str)] {
    if op == "equal" {
        &[
            ("exact date", ""),
            ("today", "0"),
            ("tomorrow", "1"),
            ("yesterday", "-1"),
            ("in the past week", "TODAY()-7"),
            ("in the past month", "EDATE(TODAY(),-1)"),
            ("in the past year", "EDATE(TODAY(),-12)"),
        ]
    } else {
        &[
            ("exact date", ""),
            ("today", "TODAY()"),
            ("tomorrow", "TODAY()+1"),
            ("yesterday", "TODAY()-1"),
            ("one week ago", "TODAY()-7"),
            ("one month ago", "EDATE(TODAY(),-1)"),
            ("one year ago", "EDATE(TODAY(),-12)"),
        ]
    }
}

/// The custom formula for a date choice, written for the range's top-left cell `tl`. Blank and
/// text cells never match.
fn date_formula(op: &str, when: usize, tl: &str) -> Option<String> {
    let (_, f) = *date_choices(op).get(when)?;
    if f.is_empty() {
        return None;
    }
    Some(match (op, when) {
        ("equal", 1..=3) => format!(
            "=AND(ISNUMBER({tl}),INT({tl})=TODAY(){})",
            if f == "0" {
                String::new()
            } else if f.starts_with('-') {
                f.to_string()
            } else {
                format!("+{f}")
            }
        ),
        ("equal", _) => format!("=AND(ISNUMBER({tl}),{tl}>={f},{tl}<=TODAY())"),
        ("less", _) => format!("=AND(ISNUMBER({tl}),{tl}<{f})"),
        _ => format!("=AND(ISNUMBER({tl}),{tl}>={f}+1)"),
    })
}

/// A rule made from a date choice comes back as a custom formula: recognise it again.
fn read_date_choice(d: &mut Draft) {
    let Some(tl) = top_left(&d.range) else { return };
    let formula = d.value1.trim().to_string();
    for (ci, (_, kind, op, _)) in CONDITIONS.iter().enumerate() {
        if *kind != "date" {
            continue;
        }
        for w in 1..date_choices(op).len() {
            if date_formula(op, w, &tl).is_some_and(|f| f.eq_ignore_ascii_case(&formula)) {
                d.condition = ci;
                d.when = w;
                d.value1.clear();
                return;
            }
        }
    }
}

fn top_left(range: &str) -> Option<String> {
    crate::refs::top_left(range)
}

/// Google Sheets reads a whole column in a custom formula (`=$A:$A=12`) as that column's cell in
/// each row of the range, so one rule can colour whole rows; GridCraft (like Excel) wants the
/// first row's cell (`=$A1=12`). `top_row` is the range's first row (1-based). Text in quotes
/// is left alone.
pub(crate) fn sheets_rows(formula: &str, top_row: u32) -> String {
    let re = regex::Regex::new(r"(\$?)([A-Za-z]{1,3}):(\$?)([A-Za-z]{1,3})\b").expect("valid pattern");
    formula
        .split('"')
        .enumerate()
        .map(|(i, part)| {
            if i % 2 == 1 {
                return part.to_string();
            }
            re.replace_all(part, |c: &regex::Captures| {
                if c[2].eq_ignore_ascii_case(&c[4]) { format!("{}{}{top_row}", &c[1], c[2].to_ascii_uppercase()) } else { c[0].to_string() }
            })
            .into_owned()
        })
        .collect::<Vec<_>>()
        .join("\"")
}

pub(crate) fn date_number(s: &str) -> Option<i64> {
    let parts: Vec<i64> = s.trim().split(['/', '-', '.']).map(|x| x.trim().parse().ok()).collect::<Option<_>>()?;
    let [d, m, y] = parts[..] else { return None };
    if !(1..=31).contains(&d) || !(1..=12).contains(&m) || y < 1900 {
        return None;
    }
    // Days from civil (Howard Hinnant), then shift the epoch to 1899-12-30.
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468 + 25_569)
}

fn rule_json(d: &Draft) -> Result<Json, String> {
    if d.range.trim().is_empty() {
        return Err("Choose the cells the rule applies to.".into());
    }
    if d.scale {
        let mut stops = Vec::new();
        for (i, p) in d.points.iter().enumerate() {
            let kind = POINT_KINDS[p.kind].1;
            if kind.is_empty() {
                continue;
            }
            let mut s = json!({ "type": kind, "color": hex(p.colour) });
            if matches!(kind, "num" | "percent" | "percentile") {
                let v: f64 = p.value.trim().parse().map_err(|_| format!("{} needs a number.", ["Minpoint", "Midpoint", "Maxpoint"][i]))?;
                s["value"] = json!(v);
            }
            stops.push(s);
        }
        return Ok(json!({ "type": "colorScale", "stops": stops }));
    }
    let (_, kind, op, boxes) = CONDITIONS[d.condition];
    if boxes >= 1 && d.value1.trim().is_empty() && !(kind == "date" && d.when > 0) {
        return Err("Enter a value for the rule.".into());
    }
    if boxes >= 2 && d.value2.trim().is_empty() {
        return Err("Enter both values.".into());
    }
    let mut font = serde_json::Map::new();
    if d.bold {
        font.insert("bold".into(), json!(true));
    }
    if d.italic {
        font.insert("italic".into(), json!(true));
    }
    if d.underline {
        font.insert("underline".into(), json!("Single"));
    }
    if d.strike {
        font.insert("strike".into(), json!(true));
    }
    if let Some(t) = d.text {
        font.insert("color".into(), json!(hex(t)));
    }
    let mut style = json!({ "font": font });
    if let Some(f) = d.fill {
        style["fill"] = json!(hex(f));
    }
    let v1 = d.value1.trim().to_string();
    let rule = match kind {
        "date" if d.when > 0 => {
            let tl = top_left(&d.range).ok_or("Choose the cells the rule applies to.")?;
            json!({ "type": "expression", "formula": date_formula(op, d.when, &tl).unwrap_or_default(), "style": style })
        }
        "date" => {
            let n = date_number(&v1).ok_or("Enter the date as dd/mm/yyyy.")?;
            json!({ "type": "cellIs", "operator": op, "value": n.to_string(), "style": style })
        }
        "cellIs" if boxes == 2 => json!({ "type": "cellIs", "operator": op, "value": v1, "value2": d.value2.trim(), "style": style }),
        "cellIs" => json!({ "type": "cellIs", "operator": op, "value": v1, "style": style }),
        "expression" => {
            let row = crate::refs::area(d.range.split(',').next().unwrap_or("")).map_or(1, |(_, r)| r.start.row + 1);
            json!({ "type": "expression", "formula": sheets_rows(&v1, row), "style": style })
        }
        "blanks" | "noBlanks" => json!({ "type": kind, "style": style }),
        _ => json!({ "type": kind, "text": v1, "style": style }),
    };
    Ok(rule)
}

/// The words on a rule's card, as Sheets writes them.
fn describe(rule: &CfRule) -> String {
    let op = |o: &CfOperator| match o {
        CfOperator::Between => "Is between",
        CfOperator::NotBetween => "Is not between",
        CfOperator::Equal => "Is equal to",
        CfOperator::NotEqual => "Is not equal to",
        CfOperator::Greater => "Greater than",
        CfOperator::Less => "Less than",
        CfOperator::GreaterOrEqual => "Greater than or equal to",
        CfOperator::LessOrEqual => "Less than or equal to",
    };
    match rule {
        CfRule::CellIs { op: o, a, b: Some(b), .. } => format!("{} {a} and {b}", op(o)),
        CfRule::CellIs { op: o, a, .. } => format!("{} {a}", op(o)),
        CfRule::Expression { formula, .. } => format!("Custom formula is ={formula}"),
        CfRule::ContainsText { text, .. } => format!("Text contains \"{text}\""),
        CfRule::NotContainsText { text, .. } => format!("Text does not contain \"{text}\""),
        CfRule::BeginsWith { text, .. } => format!("Text starts with \"{text}\""),
        CfRule::EndsWith { text, .. } => format!("Text ends with \"{text}\""),
        CfRule::Blanks { .. } => "Cell is empty".into(),
        CfRule::NoBlanks { .. } => "Cell is not empty".into(),
        CfRule::ColorScale { .. } => "Colour scale".into(),
        CfRule::Errors { .. } => "Cell has an error".into(),
        CfRule::Duplicate { .. } => "Duplicate values".into(),
        CfRule::Unique { .. } => "Unique values".into(),
        CfRule::DataBar { .. } => "Data bar".into(),
        _ => "Rule".into(),
    }
}

fn rule_style(rule: &CfRule) -> Option<&gridcraft_engine::model::Style> {
    match rule {
        CfRule::CellIs { style, .. }
        | CfRule::Expression { style, .. }
        | CfRule::ContainsText { style, .. }
        | CfRule::NotContainsText { style, .. }
        | CfRule::BeginsWith { style, .. }
        | CfRule::EndsWith { style, .. }
        | CfRule::Blanks { style, .. }
        | CfRule::NoBlanks { style, .. }
        | CfRule::Errors { style, .. }
        | CfRule::NoErrors { style, .. }
        | CfRule::Duplicate { style, .. }
        | CfRule::Unique { style, .. }
        | CfRule::Top10 { style, .. }
        | CfRule::AboveAverage { style, .. }
        | CfRule::TimePeriod { style, .. } => Some(style),
        _ => None,
    }
}

fn paint_swatch(p: &egui::Painter, r: egui::Rect, rule: &CfRule, theme_colours: &gridcraft_engine::model::Theme) {
    if let CfRule::ColorScale { stops } = rule {
        let cols: Vec<Color32> = stops.iter().map(|(_, c)| rgb_of(c, theme_colours).map_or(Color32::WHITE, c32)).collect();
        if cols.len() >= 2 {
            paint_gradient(p, r, &cols);
        }
        return;
    }
    let style = rule_style(rule);
    let fill = style.and_then(|s| rgb_of(&s.fill.fg, theme_colours).or_else(|| rgb_of(&s.fill.bg, theme_colours))).map_or(Color32::WHITE, c32);
    let text = style.and_then(|s| rgb_of(&s.font.color, theme_colours)).map_or(theme::TEXT, c32);
    p.rect_filled(r, 2.0, fill);
    p.text(r.center(), egui::Align2::CENTER_CENTER, "123", egui::FontId::proportional(12.0), text);
}

/// A draft for editing an existing rule; None for kinds the panel can't edit (they can still
/// be removed).
fn draft_from(rule: &CfRule, theme_colours: &gridcraft_engine::model::Theme) -> Option<Draft> {
    let mut d = Draft::new(String::new());
    if let CfRule::ColorScale { stops } = rule {
        d.scale = true;
        let point = |(k, c): &(CfValueKind, Color)| {
            let (kind, value) = match k {
                CfValueKind::Min => (1, String::new()),
                CfValueKind::Max => (2, String::new()),
                CfValueKind::Number(v) => (3, v.to_string()),
                CfValueKind::Percent(v) => (4, v.to_string()),
                CfValueKind::Percentile(v) => (5, v.to_string()),
                CfValueKind::Formula(f) => (3, f.clone()),
            };
            Point { kind, value, colour: rgb_of(c, theme_colours).unwrap_or(0xFFFFFF) }
        };
        d.points[0] = point(stops.first()?);
        d.points[2] = point(stops.last()?);
        if stops.len() == 3 {
            d.points[1] = point(&stops[1]);
        } else {
            d.points[1].kind = 0;
        }
        return Some(d);
    }
    let style = rule_style(rule)?;
    let find = |kind: &str, op: &str| CONDITIONS.iter().position(|(_, k, o, _)| *k == kind && *o == op);
    let (cond, v1, v2) = match rule {
        CfRule::CellIs { op, a, b, .. } => {
            let o = match op {
                CfOperator::Between => "between",
                CfOperator::NotBetween => "notBetween",
                CfOperator::Equal => "equal",
                CfOperator::NotEqual => "notEqual",
                CfOperator::Greater => "greater",
                CfOperator::Less => "less",
                CfOperator::GreaterOrEqual => "greaterOrEqual",
                CfOperator::LessOrEqual => "lessOrEqual",
            };
            // "Is equal to" (14) rather than "Text is exactly" (6), the first match.
            let i = if o == "equal" { Some(14) } else { find("cellIs", o) };
            (i?, a.trim_matches('"').to_string(), b.clone().unwrap_or_default())
        }
        CfRule::Expression { formula, .. } => (find("expression", "")?, format!("={formula}"), String::new()),
        CfRule::ContainsText { text, .. } => (find("containsText", "")?, text.clone(), String::new()),
        CfRule::NotContainsText { text, .. } => (find("notContainsText", "")?, text.clone(), String::new()),
        CfRule::BeginsWith { text, .. } => (find("beginsWith", "")?, text.clone(), String::new()),
        CfRule::EndsWith { text, .. } => (find("endsWith", "")?, text.clone(), String::new()),
        CfRule::Blanks { .. } => (0, String::new(), String::new()),
        CfRule::NoBlanks { .. } => (1, String::new(), String::new()),
        _ => return None,
    };
    d.condition = cond;
    d.value1 = v1;
    d.value2 = v2;
    d.bold = style.font.bold;
    d.italic = style.font.italic;
    d.underline = style.font.underline != Underline::None;
    d.strike = style.font.strike;
    d.text = rgb_of(&style.font.color, theme_colours);
    d.fill = rgb_of(&style.fill.fg, theme_colours).or_else(|| rgb_of(&style.fill.bg, theme_colours));
    Some(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_become_spreadsheet_numbers() {
        assert_eq!(date_number("01/01/1900"), Some(2));
        assert_eq!(date_number("25/12/2026"), Some(46381));
        assert_eq!(date_number("31/13/2026"), None);
    }

    #[test]
    fn date_choices_round_trip() {
        for (op, ci) in [("equal", 7), ("less", 8), ("greater", 9)] {
            for w in 1..date_choices(op).len() {
                let mut d = Draft::new("B2:B9".into());
                d.condition = ci;
                d.when = w;
                let rule = rule_json(&d).unwrap();
                let mut back = Draft::new("B2:B9".into());
                back.condition = 18;
                back.value1 = format!("={}", rule["formula"].as_str().unwrap().trim_start_matches('='));
                read_date_choice(&mut back);
                assert_eq!((back.condition, back.when), (ci, w), "{op} {w}");
            }
        }
    }

    #[test]
    fn whole_columns_mean_this_row() {
        assert_eq!(sheets_rows("=$A:$A=12", 1), "=$A1=12");
        assert_eq!(sheets_rows("=AND($B:$B>3, C:C=\"x:y\")", 4), "=AND($B4>3, C4=\"x:y\")");
        assert_eq!(sheets_rows("=SUM(A:C)>0", 1), "=SUM(A:C)>0");
    }

    #[test]
    fn whole_rows_light_up_from_one_column() {
        let mut s = gridcraft_engine::Session::new();
        s.execute("file.new", json!({})).unwrap();
        for (c, v) in [("A1", "45"), ("A4", "12"), ("A5", "45"), ("A10", "12")] {
            s.execute("cell.set", json!({"cell": c, "input": v})).unwrap();
        }
        let mut d = Draft::new("A1:F13".into());
        d.condition = 18;
        d.value1 = "=$A:$A=12".into();
        s.execute("home.conditionalFormat", json!({"range": "A1:F13", "rule": rule_json(&d).unwrap()})).unwrap();
        let doc = s.active().unwrap();
        let sh = doc.wb.active().unwrap();
        let hit = |a1: &str| {
            gridcraft_engine::calc::evaluate(
                &doc.wb,
                0,
                gridcraft_engine::core::CellRef::parse(a1).unwrap(),
                &sheets_rows("=$A:$A=12", 1).replace("$A1", &format!("$A{}", &a1[1..])),
            )
        };
        assert_eq!(hit("F4"), gridcraft_engine::core::Value::Bool(true));
        assert_eq!(hit("C5"), gridcraft_engine::core::Value::Bool(false));
        assert!(matches!(&sh.cond_formats[0].rule, gridcraft_engine::model::CfRule::Expression { formula, .. } if formula.contains("$A1")));
    }

    #[test]
    fn rules_turn_into_gridcraft_json() {
        let mut d = Draft::new("A1:A10".into());
        d.condition = 10;
        d.value1 = "5".into();
        let r = rule_json(&d).unwrap();
        assert_eq!(r["type"], "cellIs");
        assert_eq!(r["operator"], "greater");
        assert_eq!(r["style"]["fill"], "#B7E1CD");
        d.scale = true;
        d.points[1].kind = 5;
        d.points[1].value = "50".into();
        let s = rule_json(&d).unwrap();
        assert_eq!(s["stops"].as_array().unwrap().len(), 3);
        assert_eq!(s["stops"][1]["type"], "percentile");
    }
}
