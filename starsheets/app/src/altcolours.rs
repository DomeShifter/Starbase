//! Format › Alternating colours, as in Google Sheets: a side panel that bands a range's rows in
//! two colours, with an optional header and footer colour, from a set of preset styles or your
//! own colours, and "Remove alternating colours".
//!
//! Sheets keeps banding as its own kind of formatting; .xlsx has nothing like it for a plain
//! range (only tables, which rewrite the header). So the bands are conditional-formatting rules
//! of a fixed shape that Excel and other apps draw the same way, kept after the sheet's own
//! rules so those win, and left out of the conditional formatting panel (docs/decisions.md):
//!   header  `ROW()=ROW($A$1)` on the range, at its top row
//!   footer  `ROW()=ROW($A$9)` at its bottom row
//!   bands   `ISEVEN(ROW()-ROW($A$2))` (colour 1) and `ISODD(…)` (colour 2) from the first body row
//! A change rewrites the sheet's rule list in one step (`home.manageRules` `all`, StarSheets 0098).

use egui::{Color32, CornerRadius, RichText, Sense, Stroke, Ui, vec2};
use gridcraft_engine::core::{CellRef, RangeRef};
use gridcraft_engine::model::style::{Fill, Style};
use gridcraft_engine::model::{CfRule, Color, CondFormat, Sheet};
use gridcraft_ui_egui::SheetApp;
use serde_json::{Value as Json, json};

use crate::{house, theme};

/// Asks the panel to open (egui temp data).
pub const REQUEST: &str = "starsheets_alternating";
const PANEL_W: f32 = 320.0;

/// One banding: its range, and header / colour 1 / colour 2 / footer (0xRRGGBB; no header or
/// footer when None).
#[derive(Clone, Debug, PartialEq)]
pub struct Banding {
    pub range: RangeRef,
    pub header: Option<u32>,
    pub band1: u32,
    pub band2: u32,
    pub footer: Option<u32>,
}

/// Google's default styles: header, colour 1, colour 2, footer.
pub const PRESETS: &[(u32, u32, u32, u32)] = &[
    (0xBDBDBD, 0xFFFFFF, 0xF3F3F3, 0xDEDEDE),
    (0x5B95F9, 0xFFFFFF, 0xE8F0FE, 0xACC9FE),
    (0x57BB8A, 0xFFFFFF, 0xE7F9EF, 0xB7E1CD),
    (0xF7CB4D, 0xFFFFFF, 0xFEF8E3, 0xFCE8B2),
    (0xF46524, 0xFFFFFF, 0xFFE6DD, 0xFFCCBC),
    (0x46BDC6, 0xFFFFFF, 0xE4F7F8, 0xA8E2E6),
    (0x8E7CC3, 0xFFFFFF, 0xEFEBF9, 0xD0C7EA),
    (0xE67C73, 0xFFFFFF, 0xFCE8E6, 0xF4C7C3),
];

/// `$A$1` for a cell.
fn abs(c: CellRef) -> String {
    format!("${}${}", gridcraft_engine::core::col_to_letters(c.col), c.row + 1)
}

/// The cell named in `ROW($A$1)` inside a formula of ours.
fn row_ref(formula: &str) -> Option<CellRef> {
    let inner = formula.split("ROW($").nth(1)?.split(')').next()?;
    CellRef::parse(&inner.replace('$', ""))
}

/// What one of our rules is: 'h' header or footer (by the row it names), '1' / '2' a band.
fn kind(cf: &CondFormat) -> Option<(char, CellRef, u32)> {
    let CfRule::Expression { formula, style } = &cf.rule else { return None };
    let f = formula.trim_start_matches('=');
    let k = if f.starts_with("ROW()=ROW($") && f.ends_with(')') {
        'h'
    } else if f.starts_with("ISEVEN(ROW()-ROW($") && f.ends_with("))") {
        '1'
    } else if f.starts_with("ISODD(ROW()-ROW($") && f.ends_with("))") {
        '2'
    } else {
        return None;
    };
    let Color::Rgb(rgb) = style.fill.fg else { return None };
    if cf.ranges.len() != 1 {
        return None;
    }
    Some((k, row_ref(f)?, rgb))
}

/// Whether a conditional-formatting rule is one of Alternating colours' (the conditional
/// formatting panel leaves these out).
pub fn is_ours(cf: &CondFormat) -> bool {
    kind(cf).is_some()
}

/// Every banding on a sheet, read back from its rules.
pub fn bandings(sh: &Sheet) -> Vec<Banding> {
    let mut out: Vec<Banding> = Vec::new();
    for cf in &sh.cond_formats {
        let Some((k, at, rgb)) = kind(cf) else { continue };
        let range = cf.ranges[0];
        let i = match out.iter().position(|b| b.range == range) {
            Some(i) => i,
            None => {
                out.push(Banding { range, header: None, band1: 0xFFFFFF, band2: 0xFFFFFF, footer: None });
                out.len() - 1
            }
        };
        let b = &mut out[i];
        match k {
            '1' => b.band1 = rgb,
            '2' => b.band2 = rgb,
            _ if at.row == range.start.row => b.header = Some(rgb),
            _ => b.footer = Some(rgb),
        }
    }
    out
}

fn rule(range: RangeRef, formula: String, rgb: u32) -> CondFormat {
    let style = Style { fill: Fill::solid(Color::Rgb(rgb)), ..Style::default() };
    CondFormat { ranges: vec![range], rule: CfRule::Expression { formula, style: Box::new(style) }, priority: 0, stop_if_true: false }
}

/// The rules for one banding: header and footer first (they win over the bands).
pub fn rules(b: &Banding) -> Vec<CondFormat> {
    let r = b.range;
    let mut out = Vec::new();
    let mut body = r.start.row;
    if let Some(h) = b.header {
        out.push(rule(r, format!("ROW()=ROW({})", abs(r.start)), h));
        body += 1;
    }
    if let Some(f) = b.footer.filter(|_| r.end.row > r.start.row) {
        out.push(rule(r, format!("ROW()=ROW({})", abs(CellRef::new(r.end.row, r.start.col))), f));
    }
    let first = abs(CellRef::new(body, r.start.col));
    out.push(rule(r, format!("ISEVEN(ROW()-ROW({first}))"), b.band1));
    out.push(rule(r, format!("ISODD(ROW()-ROW({first}))"), b.band2));
    out
}

/// Puts `new` on the active sheet in place of the banding over `old` (or adds it), or with
/// `new` None removes the banding over `old`. One undo step.
pub fn apply(app: &mut SheetApp, old: Option<RangeRef>, new: Option<&Banding>) -> Result<(), String> {
    let sh = app.session.active().and_then(|d| d.wb.active()).ok_or("There's no sheet open.")?;
    let gone = |cf: &CondFormat| is_ours(cf) && (Some(cf.ranges[0]) == old || new.is_some_and(|n| cf.ranges[0] == n.range));
    let mut list: Vec<CondFormat> = sh.cond_formats.iter().filter(|cf| !gone(cf)).cloned().collect();
    if let Some(n) = new {
        list.extend(rules(n));
    }
    if list == sh.cond_formats {
        return Ok(());
    }
    let all = serde_json::to_value(&list).map_err(|e| e.to_string())?;
    app.run("home.manageRules", json!({ "all": all })).map(|_| ()).map_err(|e| crate::messages::plain_error(&gridcraft_ui_egui::clean_error(&e)))
}

/// The banding the selection is in, or a new one for the selection (its data when one cell is
/// selected) with the first preset and a header.
fn for_selection(app: &SheetApp) -> Option<(Banding, bool)> {
    let d = app.session.active()?;
    let sh = d.wb.active()?;
    let sel = d.selection.current();
    if let Some(b) = bandings(sh).into_iter().find(|b| b.range.intersection(&sel).is_some()) {
        return Some((b, true));
    }
    let range = if d.selection.is_single_cell() { sh.used_range().unwrap_or(sel) } else { sel.intersection(&sh.used_range()?).unwrap_or(sel) };
    let (h, c1, c2, _) = PRESETS[0];
    Some((Banding { range, header: Some(h), band1: c1, band2: c2, footer: None }, false))
}

/// The test hook's view: each banding on the active sheet.
#[cfg(target_arch = "wasm32")]
pub fn state(sh: &Sheet) -> Json {
    let hex = |c: u32| format!("#{c:06X}");
    bandings(sh)
        .iter()
        .map(|b| json!({ "range": b.range.a1(), "header": b.header.map(hex), "band1": hex(b.band1), "band2": hex(b.band2), "footer": b.footer.map(hex) }))
        .collect()
}

/// Runs `panel:alternatingColours` with parameters (the test hook): `{"range"?, "preset"?: 0-7,
/// "header"?: bool, "footer"?: bool}` applies, `{"remove": true}` removes the selection's.
pub fn run(app: &mut SheetApp, p: &Json) -> Result<(), String> {
    let (mut b, existing) = for_selection(app).ok_or("There's no sheet open.")?;
    let old = existing.then_some(b.range);
    if p.get("remove").and_then(Json::as_bool) == Some(true) {
        return if existing { apply(app, old, None) } else { Err("There are no alternating colours here.".into()) };
    }
    if let Some(r) = p.get("range").and_then(Json::as_str) {
        b.range = RangeRef::parse(&crate::refs::plain(r)).ok_or("That range isn't valid.")?;
    }
    let preset = PRESETS[p.get("preset").and_then(Json::as_u64).unwrap_or(0) as usize % PRESETS.len()];
    (b.band1, b.band2) = (preset.1, preset.2);
    b.header = (p.get("header").and_then(Json::as_bool) != Some(false)).then_some(preset.0);
    b.footer = (p.get("footer").and_then(Json::as_bool) == Some(true)).then_some(preset.3);
    apply(app, old, Some(&b))
}

#[derive(Default)]
pub struct AltColoursPanel {
    pub open: bool,
    pub just_opened: bool,
    /// What is on the sheet now (the panel's changes go straight on, as in Sheets).
    shown: Option<Banding>,
    range_text: String,
    error: Option<String>,
}

impl AltColoursPanel {
    pub fn close(&mut self) {
        self.open = false;
        self.shown = None;
        self.error = None;
    }

    pub fn intercept(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        self.just_opened = false;
        if ctx.data_mut(|m| m.remove_temp::<bool>(egui::Id::unique(REQUEST))).is_none() {
            return;
        }
        self.open = true;
        self.just_opened = true;
        self.error = None;
        match for_selection(app) {
            Some((b, existing)) => {
                self.range_text = b.range.a1();
                if !existing && let Err(e) = apply(app, None, Some(&b)) {
                    self.error = Some(e);
                }
                self.shown = Some(b);
            }
            None => self.close(),
        }
    }

    pub fn ui(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        if !self.open {
            return;
        }
        let Some(mut b) = self.shown.clone() else { return self.close() };
        let old = b.range;
        let mut remove = false;
        egui::Panel::right("starsheets_alternating")
            .exact_size(PANEL_W + 32.0 + house::SCROLL_ROOM)
            .resizable(false)
            .frame(house::panel_frame())
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Alternating colours").size(16.0).color(theme::TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if house::close_button(ui) {
                            self.open = false;
                        }
                    });
                });
                ui.add_space(house::S);
                egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(house::panel_scroll_height(ui)).show(ui, |ui| {
                    house::form_style(ui);
                    ui.set_width(PANEL_W);
                    house::label(ui, "Apply to range", "");
                    let r = ui.add_sized([PANEL_W, house::FIELD_HEIGHT], house::field(&mut self.range_text, house::BODY));
                    if r.lost_focus() || (r.changed() && RangeRef::parse(&crate::refs::plain(&self.range_text)).is_some()) {
                        match RangeRef::parse(&crate::refs::plain(&self.range_text)) {
                            Some(range) => b.range = range,
                            None if r.lost_focus() => self.range_text = b.range.a1(),
                            None => {}
                        }
                    }
                    ui.add_space(house::L);
                    house::caps(ui, "Styles");
                    ui.add_space(house::XS);
                    ui.horizontal(|ui| {
                        let mut header = b.header.is_some();
                        if ui.checkbox(&mut header, "Header").changed() {
                            b.header = header.then(|| PRESETS.iter().find(|p| p.2 == b.band2).map_or(0xBDBDBD, |p| p.0));
                        }
                        ui.add_space(house::L);
                        let mut footer = b.footer.is_some();
                        if ui.checkbox(&mut footer, "Footer").changed() {
                            b.footer = footer.then(|| PRESETS.iter().find(|p| p.2 == b.band2).map_or(0xDEDEDE, |p| p.3));
                        }
                    });
                    ui.add_space(house::M);
                    house::note(ui, "Default styles");
                    ui.add_space(house::XS);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(house::S, house::S);
                        for p in PRESETS {
                            let picked = b.band2 == p.2 && b.band1 == p.1;
                            if preset_swatch(ui, p, picked).clicked() {
                                b.band1 = p.1;
                                b.band2 = p.2;
                                b.header = b.header.map(|_| p.0);
                                b.footer = b.footer.map(|_| p.3);
                            }
                        }
                    });
                    ui.add_space(house::M);
                    house::note(ui, "Custom style");
                    ui.add_space(house::XS);
                    let colour_row = |ui: &mut Ui, label: &str, now: u32| -> Option<u32> {
                        let mut out = None;
                        ui.horizontal(|ui| {
                            ui.add_sized([120.0, house::ROW_HEIGHT], egui::Label::new(RichText::new(label).size(house::BODY).color(theme::TEXT)));
                            if let Some(Some(rgb)) = crate::cfpanel::colour_pick(ui, label, Some(now), "format_color_fill", label) {
                                out = Some(rgb);
                            }
                        });
                        out
                    };
                    if let Some(h) = b.header
                        && let Some(c) = colour_row(ui, "Header", h)
                    {
                        b.header = Some(c);
                    }
                    if let Some(c) = colour_row(ui, "Colour 1", b.band1) {
                        b.band1 = c;
                    }
                    if let Some(c) = colour_row(ui, "Colour 2", b.band2) {
                        b.band2 = c;
                    }
                    if let Some(f) = b.footer
                        && let Some(c) = colour_row(ui, "Footer", f)
                    {
                        b.footer = Some(c);
                    }
                    if let Some(e) = &self.error {
                        ui.add_space(house::S);
                        ui.label(RichText::new(e).size(house::NOTE).color(house::ERROR));
                    }
                    ui.add_space(house::L);
                    ui.horizontal(|ui| {
                        if ui.add(house::link("Remove alternating colours")).clicked() {
                            remove = true;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(house::primary("Done")).clicked() {
                                self.open = false;
                            }
                        });
                    });
                });
            });
        if remove {
            match apply(app, Some(old), None) {
                Ok(()) => self.close(),
                Err(e) => self.error = Some(e),
            }
            return;
        }
        if self.shown.as_ref() != Some(&b) {
            match apply(app, Some(old), Some(&b)) {
                Ok(()) => self.error = None,
                Err(e) => self.error = Some(e),
            }
            self.shown = Some(b);
        }
        if !self.open {
            self.close();
        }
    }
}

/// A default style: a small table drawn in its colours (header row, then the two bands), with a
/// blue ring when it is the one in use.
fn preset_swatch(ui: &mut Ui, p: &(u32, u32, u32, u32), picked: bool) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(vec2(64.0, 40.0), Sense::click());
    let painter = ui.painter();
    let rows = [p.0, p.1, p.2, p.1];
    let h = rect.height() / rows.len() as f32;
    for (i, c) in rows.iter().enumerate() {
        let row = egui::Rect::from_min_size(rect.min + vec2(0.0, h * i as f32), vec2(rect.width(), h));
        painter.rect_filled(row, 0.0, crate::cfpanel::c32(*c));
    }
    let ring = if picked { Stroke::new(2.0, house::BLUE) } else { Stroke::new(1.0, theme::DIVIDER) };
    painter.rect_stroke(rect, CornerRadius::same(house::FIELD_RADIUS), ring, egui::StrokeKind::Outside);
    if r.hovered() {
        painter.rect_filled(rect, CornerRadius::same(house::FIELD_RADIUS), Color32::from_black_alpha(20));
    }
    r.on_hover_text("Use this style")
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridcraft_engine::Session;
    use gridcraft_ui_egui::Services;

    fn app() -> SheetApp {
        let mut app = SheetApp::new(Session::new(), Services::default());
        app.session.execute("file.new", json!({})).unwrap();
        let rows = json!([["Name", "Team"], ["Ann", "Red"], ["Bob", "Blue"], ["Cy", "Red"], ["Total", "3"]]);
        app.session.execute("range.setValues", json!({"range": "B2", "values": rows})).unwrap();
        app
    }

    fn sheet(app: &SheetApp) -> &Sheet {
        app.session.active().unwrap().wb.active().unwrap()
    }

    #[test]
    fn bands_a_range_and_reads_it_back_then_removes_it_in_one_step_each() {
        let mut a = app();
        a.session.execute("selection.set", json!({"range": "C3"})).unwrap();
        let undo = a.session.active().unwrap().undo.len();
        run(&mut a, &json!({"preset": 1, "footer": true})).unwrap();
        assert_eq!(a.session.active().unwrap().undo.len(), undo + 1, "one undo step");
        let b = bandings(sheet(&a));
        let (h, c1, c2, f) = PRESETS[1];
        assert_eq!(b, vec![Banding { range: RangeRef::parse("B2:C6").unwrap(), header: Some(h), band1: c1, band2: c2, footer: Some(f) }]);
        assert!(sheet(&a).cond_formats.iter().all(is_ours));
        let formulas: Vec<String> = rules(&b[0])
            .iter()
            .map(|cf| match &cf.rule {
                CfRule::Expression { formula, .. } => formula.clone(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(formulas, ["ROW()=ROW($B$2)", "ROW()=ROW($B$6)", "ISEVEN(ROW()-ROW($B$3))", "ISODD(ROW()-ROW($B$3))"]);

        // Again in the range: changes it in place (no second banding).
        run(&mut a, &json!({"preset": 2, "header": false})).unwrap();
        let b = bandings(sheet(&a));
        assert_eq!(b.len(), 1);
        assert_eq!((b[0].header, b[0].band2), (None, PRESETS[2].2));

        run(&mut a, &json!({"remove": true})).unwrap();
        assert!(sheet(&a).cond_formats.is_empty());
        assert!(run(&mut a, &json!({"remove": true})).is_err());
    }

    #[test]
    fn other_rules_stay_and_win() {
        let mut a = app();
        let mine = json!({"type": "cellIs", "operator": "equal", "value": "Red", "preset": "lightRedFill"});
        a.session.execute("home.conditionalFormat", json!({"range": "C3:C5", "rule": mine})).unwrap();
        a.session.execute("selection.set", json!({"range": "B2:C6"})).unwrap();
        run(&mut a, &json!({})).unwrap();
        let cfs = &sheet(&a).cond_formats;
        assert_eq!(cfs.len(), 4);
        assert!(!is_ours(&cfs[0]), "the sheet's own rule keeps first place");
    }
}
