//! Data validation as a side panel, the way Google Sheets does it (docs/sheets-dv.md): a list of
//! rule cards, and an editor with "Apply to range", "Criteria", drop-down items with colours and
//! "Advanced options" (help text, warning or reject, display style). Rules are stored by
//! GridCraft (`data.validation`); Sheets' text and tick-box criteria become custom formulas, and
//! drop-down item colours become ordinary conditional-formatting rules, so both survive in .xlsx.
//! Which criterion a rule was made with, and which conditional-formatting rules are its item
//! colours, is kept in StarSheets' section of the file (hostdata.rs); files without it are read
//! back from the formulas' shapes as before.
//! Like Sheets, every change is applied to the sheet at once. The live preview takes its own last
//! version off with ordinary commands (never undo: with live editing undo is the server's, and a
//! run of undos and edits in one frame came back out of step, so other rules vanished and
//! reappeared: to-do row 14).

use egui::{Color32, RichText, Sense, Stroke, Ui, vec2};
use gridcraft_engine::model::{CfOperator, CfRule, DropdownDisplay, ErrorStyle, Validation, ValidationKind};
use gridcraft_ui_egui::SheetApp;
use serde_json::{Value as Json, json};

use crate::cfpanel::{c32, date_number, hex, rgb_of};
use crate::theme;

const GREEN: Color32 = crate::house::GREEN;
const PANEL_W: f32 = 320.0;
/// Asks the panel to open with a new drop-down rule (Insert › Drop-down), through egui's memory.
pub const DROPDOWN_REQUEST: &str = "starsheets_dv_dropdown";

#[derive(Clone, Copy, PartialEq)]
enum Crit {
    Dropdown,
    DropdownRange,
    TextContains,
    TextNotContains,
    TextExactly,
    Email,
    Url,
    ValidDate,
    /// Date is / before / on or before / after / on or after / between / not between.
    Date(&'static str),
    /// Greater than … not between.
    Number(&'static str),
    TickBox,
    Custom,
}

/// Google Sheets' criteria, in its order; `true` = a divider after it.
const CRITERIA: &[(&str, Crit, bool)] = &[
    ("Drop-down", Crit::Dropdown, false),
    ("Drop-down (from a range)", Crit::DropdownRange, true),
    ("Text contains", Crit::TextContains, false),
    ("Text does not contain", Crit::TextNotContains, false),
    ("Text is exactly", Crit::TextExactly, false),
    ("Text is valid email", Crit::Email, false),
    ("Text is valid URL", Crit::Url, true),
    ("Is valid date", Crit::ValidDate, false),
    ("Date is", Crit::Date("equal"), false),
    ("Date is before", Crit::Date("less"), false),
    ("Date is on or before", Crit::Date("lessOrEqual"), false),
    ("Date is after", Crit::Date("greater"), false),
    ("Date is on or after", Crit::Date("greaterOrEqual"), false),
    ("Date is between", Crit::Date("between"), false),
    ("Date is not between", Crit::Date("notBetween"), true),
    ("Greater than", Crit::Number("greater"), false),
    ("Greater than or equal to", Crit::Number("greaterOrEqual"), false),
    ("Less than", Crit::Number("less"), false),
    ("Less than or equal to", Crit::Number("lessOrEqual"), false),
    ("Is equal to", Crit::Number("equal"), false),
    ("Is not equal to", Crit::Number("notEqual"), false),
    ("Is between", Crit::Number("between"), false),
    ("Is not between", Crit::Number("notBetween"), true),
    ("Tick box", Crit::TickBox, false),
    ("Custom formula is", Crit::Custom, false),
];

/// "Date is" choices: label, GridCraft operator, first and second bound (formulas).
const DATE_IS: &[(&str, &str, &str, &str)] = &[
    ("exact date", "equal", "", ""),
    ("today", "equal", "TODAY()", ""),
    ("tomorrow", "equal", "TODAY()+1", ""),
    ("yesterday", "equal", "TODAY()-1", ""),
    ("in the past week", "between", "TODAY()-7", "TODAY()"),
    ("in the past month", "between", "EDATE(TODAY(),-1)", "TODAY()"),
    ("in the past year", "between", "EDATE(TODAY(),-12)", "TODAY()"),
];
/// Choices for before / after: label, formula ("" = a typed date).
const DATE_REL: &[(&str, &str)] = &[
    ("exact date", ""),
    ("today", "TODAY()"),
    ("tomorrow", "TODAY()+1"),
    ("yesterday", "TODAY()-1"),
    ("one week ago", "TODAY()-7"),
    ("one month ago", "EDATE(TODAY(),-1)"),
    ("one year ago", "EDATE(TODAY(),-12)"),
];
/// The last valid spreadsheet date (31/12/9999).
const LAST_DATE: &str = "2958465";

/// Sheets' chip colours for drop-down items: a light row and a dark row. Each fill's text is its
/// partner in the other row (dark green text on light green, light red text on dark red), as in
/// Sheets. The first light one is the default grey (no colour rule).
const CHIP_LIGHT: [u32; 8] = [0xE8EAED, 0xBFE1F6, 0xFFCFC9, 0xFFE5A0, 0xD4EDBC, 0xE6CFF2, 0xC6DBE1, 0xFFC8AA];
const CHIP_DARK: [u32; 8] = [0x3D3D3D, 0x0A53A8, 0xB10202, 0x473822, 0x11734B, 0x5A3286, 0x215A6C, 0x753800];

/// The text colour that goes with a chip fill.
fn chip_text(fill: u32) -> u32 {
    if let Some(i) = CHIP_LIGHT.iter().position(|c| *c == fill) {
        return CHIP_DARK[i];
    }
    if let Some(i) = CHIP_DARK.iter().position(|c| *c == fill) {
        return CHIP_LIGHT[i];
    }
    0x202124
}

#[derive(Clone, Default, PartialEq)]
struct Item {
    text: String,
    colour: Option<u32>,
}

/// A rule as it is on the sheet, so it can be taken off before the new one goes on.
#[derive(Clone, Debug, PartialEq)]
struct Orig {
    /// Its ranges, in the section's form (`B2:B9,D2`).
    range: String,
    /// Drop-down items whose colour is a conditional-formatting rule ("is equal to" the item, on
    /// the same cells): found again by what they are, never by their place in the list.
    coloured: Vec<String>,
    tick_box: bool,
}

#[derive(Clone, PartialEq)]
struct Draft {
    orig: Option<Orig>,
    range: String,
    crit: usize,
    /// Which "Date is" / before-after choice (index into DATE_IS or DATE_REL).
    when: usize,
    v1: String,
    v2: String,
    items: Vec<Item>,
    help_on: bool,
    help: String,
    reject: bool,
    /// Display style, an index into DISPLAY.
    display: usize,
    /// Drop-downs: "Allow multiple selections".
    multiple: bool,
}

/// Sheets' display styles for a drop-down: label and GridCraft's name. Chip is the default for a
/// new drop-down, as in Sheets.
const DISPLAY: [(&str, &str); 3] = [("Chip", "chip"), ("Arrow", "arrow"), ("Plain text", "plain")];
const PLAIN: usize = 2;

impl Draft {
    fn new(range: String) -> Draft {
        Draft {
            orig: None,
            range,
            crit: 0,
            when: 0,
            v1: String::new(),
            v2: String::new(),
            items: vec![Item::default(), Item::default()],
            help_on: false,
            help: String::new(),
            reject: true,
            display: 0,
            multiple: false,
        }
    }
    fn crit(&self) -> Crit {
        CRITERIA[self.crit].1
    }
}

#[derive(Default)]
pub struct DvPanel {
    pub open: bool,
    draft: Option<Draft>,
    error: Option<String>,
    /// The plan last applied for the open draft (applying the same again is skipped).
    preview: Option<String>,
    /// What is on the sheet for the open draft (the rule being edited, then each preview).
    applied: Option<Orig>,
    /// Set on the frame the panel opened, so the other side panel can close.
    pub just_opened: bool,
    /// Done was pressed on this draft: only then does an incomplete rule say what's missing.
    tried: bool,
}

impl DvPanel {
    /// Opens the panel: the rule list when the sheet has rules, else a new rule, as in Sheets.
    pub fn open(&mut self, app: &SheetApp, dropdown: bool) {
        self.open = true;
        self.just_opened = true;
        self.error = None;
        self.tried = false;
        let has_rules = app.session.active().and_then(|d| d.wb.active()).is_some_and(|sh| !sh.validations.is_empty());
        self.draft = if has_rules && !dropdown { None } else { Some(Draft::new(selection_text(app))) };
        self.preview = None;
        self.applied = None;
    }

    /// Closes the panel, keeping what was applied (Sheets has no Cancel for a live rule).
    pub fn close(&mut self) {
        self.preview = None;
        self.applied = None;
        self.draft = None;
        self.open = false;
    }

    /// Takes over GridCraft's own Data Validation dialog, and Insert › Drop-down.
    pub fn intercept(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        self.just_opened = false;
        if app.dialog.as_ref().is_some_and(|d| d.command == Some("data.validation") || d.name == "dataValidation") {
            app.dialog = None;
            self.open(app, false);
        }
        if ctx.data_mut(|m| m.remove_temp::<bool>(egui::Id::unique(DROPDOWN_REQUEST))).is_some() {
            self.preview = None;
            self.open(app, true);
        }
        // "Edit rule" at the bottom of a drop-down list: that cell's rule, in the editor.
        if let Some(c) = app.grid.edit_validation.take() {
            self.open(app, false);
            let found = app.session.active().and_then(|d| {
                let sh = d.wb.active()?;
                let dv = sh.validations.iter().find(|v| v.ranges.iter().any(|r| r.contains(c)))?.clone();
                Some((dv, sh.cond_formats.clone(), d.wb.theme.clone()))
            });
            if let Some((dv, cfs, theme_colours)) = found {
                let h = crate::hostdata::read(app);
                let d = draft_from(&dv, &cfs, &theme_colours, h.dv_for(&active_sheet_name(app), &crate::hostdata::ranges_text(&dv.ranges)));
                self.applied = d.orig.clone();
                self.preview = None;
                self.draft = Some(d);
                self.just_opened = true;
            }
        }
    }

    pub fn ui(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        if !self.open {
            return;
        }
        egui::Panel::right("starsheets_dv")
            .exact_size(PANEL_W + 32.0 + crate::house::SCROLL_ROOM)
            .resizable(false)
            .frame(crate::house::panel_frame())
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Data validation rules").size(16.0).color(theme::TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new(RichText::new("✕").size(16.0)).frame(false)).on_hover_text("Close").clicked() {
                            self.close();
                        }
                    });
                });
                ui.add_space(8.0);
                egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(crate::house::panel_scroll_height(ui)).show(ui, |ui| {
                    crate::house::form_style(ui);
                    ui.set_width(PANEL_W);
                    if let Some(mut d) = self.draft.take() {
                        match self.editor(ui, &mut d) {
                            // Done: keep what is applied, back to the list.
                            Some(true) => {
                                self.tried = true;
                                self.update_preview(app, &d);
                                if self.error.is_none() {
                                    self.preview = None;
                                    self.applied = None;
                                    self.tried = false;
                                } else {
                                    self.draft = Some(d);
                                }
                            }
                            // Remove rule: take off what is on the sheet for it.
                            Some(false) => {
                                let mut h = crate::hostdata::read(app);
                                if let Some(a) = self.applied.take() {
                                    remove(app, &a, &mut h);
                                }
                                crate::hostdata::write(app, h);
                                self.preview = None;
                                self.error = None;
                                let none_left = app.session.active().and_then(|x| x.wb.active()).is_none_or(|sh| sh.validations.is_empty());
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

    /// The rule cards: range and criterion; click to edit, bin to remove.
    fn list(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        let Some((rules, cfs, theme_colours)) =
            app.session.active().and_then(|d| d.wb.active().map(|sh| (sh.validations.clone(), sh.cond_formats.clone(), d.wb.theme.clone())))
        else {
            return;
        };
        let mut remove_it = None;
        let h = crate::hostdata::read(app);
        let sheet = active_sheet_name(app);
        for (i, dv) in rules.iter().enumerate() {
            let d = draft_from(dv, &cfs, &theme_colours, h.dv_for(&sheet, &crate::hostdata::ranges_text(&dv.ranges)));
            let (r, resp) = ui.allocate_exact_size(vec2(PANEL_W, 60.0), Sense::click());
            let p = ui.painter();
            if resp.hovered() {
                p.rect_filled(r, 0.0, theme::MENU_HOVER);
            }
            p.text(r.left_center() + vec2(12.0, -10.0), egui::Align2::LEFT_CENTER, &d.range, egui::FontId::proportional(13.5), theme::TEXT);
            p.text(
                r.left_center() + vec2(12.0, 10.0),
                egui::Align2::LEFT_CENTER,
                clip(&describe(&d), 44),
                egui::FontId::proportional(12.0),
                theme::TEXT_DIM,
            );
            let bin = egui::Rect::from_center_size(egui::pos2(r.right() - 20.0, r.center().y), vec2(28.0, 28.0));
            let b = ui.interact(bin, ui.scope_id().with(("dv_bin", i)), Sense::click());
            if (resp.hovered() || b.hovered())
                && let Some(img) = crate::icons::icon("delete")
            {
                img.paint_at(ui, bin.shrink(5.0));
            }
            if b.on_hover_text("Remove rule").clicked() {
                remove_it = d.orig.clone();
                continue;
            }
            if resp.clicked() {
                self.applied = d.orig.clone();
                self.preview = None;
                self.draft = Some(d);
                self.tried = false;
            }
            ui.painter().hline(r.x_range(), r.bottom(), Stroke::new(1.0, theme::DIVIDER));
        }
        if let Some(o) = remove_it {
            let mut h = h;
            remove(app, &o, &mut h);
            crate::hostdata::write(app, h);
        }
        ui.add_space(12.0);
        if ui.add(egui::Button::new(RichText::new("+  Add rule").size(14.0).color(GREEN)).frame(false)).clicked() {
            self.draft = Some(Draft::new(selection_text(app)));
            self.applied = None;
            self.preview = None;
            self.tried = false;
        }
    }

    /// The rule editor. Returns Some(true) for Done, Some(false) for Remove rule.
    fn editor(&mut self, ui: &mut Ui, d: &mut Draft) -> Option<bool> {
        ui.label(RichText::new("Apply to range").size(14.0).strong());
        ui.add(egui::TextEdit::singleline(&mut d.range).desired_width(PANEL_W).margin(vec2(8.0, 8.0)));
        ui.add_space(12.0);
        ui.label(RichText::new("Criteria").size(14.0).strong());
        ui.add_space(4.0);
        let labels: Vec<&str> = CRITERIA.iter().map(|c| c.0).collect();
        let dividers: Vec<usize> = CRITERIA.iter().enumerate().filter(|(_, c)| c.2).map(|(i, _)| i).collect();
        crate::house::select(ui, "dv_crit", PANEL_W, &mut d.crit, &labels, &dividers);
        ui.add_space(6.0);
        criterion_inputs(ui, d);

        // Advanced options.
        ui.add_space(10.0);
        egui::CollapsingHeader::new(RichText::new("Advanced options").size(14.0).strong()).default_open(false).show(ui, |ui| {
            ui.checkbox(&mut d.help_on, "Show help text for a selected cell");
            if d.help_on {
                let hint = default_help(d);
                ui.add(egui::TextEdit::singleline(&mut d.help).hint_text(hint).desired_width(PANEL_W - 24.0).margin(vec2(8.0, 6.0)));
            }
            ui.add_space(6.0);
            ui.label(RichText::new("If the data is invalid:").color(theme::TEXT_DIM));
            ui.radio_value(&mut d.reject, false, "Show a warning");
            ui.radio_value(&mut d.reject, true, "Reject the input");
            if matches!(d.crit(), Crit::Dropdown | Crit::DropdownRange) {
                ui.add_space(6.0);
                ui.label(RichText::new("Display style").color(theme::TEXT_DIM));
                for (i, (label, _)) in DISPLAY.iter().enumerate() {
                    ui.radio_value(&mut d.display, i, *label);
                }
            }
        });

        // What's missing shows once Done is pressed (not while the rule is being filled in).
        if let Some(e) = self.error.as_ref().filter(|_| self.tried) {
            ui.add_space(6.0);
            ui.label(RichText::new(e).size(crate::house::NOTE).color(crate::house::ERROR));
        }
        ui.add_space(14.0);
        let mut out = None;
        ui.horizontal(|ui| {
            ui.add_space(PANEL_W - 230.0);
            if ui.add(crate::house::secondary("Remove rule")).clicked() {
                out = Some(false);
            }
            let done = ui.add(crate::house::primary("Done"));
            crate::house::target(ui.ctx(), "dv:done", done.rect);
            if done.clicked() {
                out = Some(true);
            }
        });
        out
    }

    /// Applies the draft as it stands (Sheets applies a rule as you build it), replacing what is
    /// on the sheet for it. An incomplete draft changes nothing and says what is missing.
    fn update_preview(&mut self, app: &mut SheetApp, d: &Draft) {
        let plan = match crate::refs::other_sheet(&d.range, &active_sheet_name(app)) {
            Some(other) => Err(format!("The range must be on this sheet, not \"{other}\".")),
            None => commands(d),
        };
        let key = format!("{:?}", plan.as_ref().map_err(|_| ()).ok());
        if self.preview.as_ref() == Some(&key) {
            self.error = plan.err();
            return;
        }
        match plan {
            Ok(cmds) => {
                self.error = None;
                let mut h = crate::hostdata::read(app);
                if let Some(a) = self.applied.take() {
                    remove(app, &a, &mut h);
                }
                for (cmd, params) in cmds {
                    if let Err(e) = app.run(cmd, params) {
                        self.error = Some(crate::messages::plain_error(&gridcraft_ui_egui::clean_error(&e)));
                        break;
                    }
                }
                let now = orig_of(d);
                if self.error.is_none() {
                    h.set_dv(meta_of(&active_sheet_name(app), d, &now.range));
                }
                self.applied = Some(now);
                crate::hostdata::write(app, h);
            }
            Err(e) => self.error = Some(e),
        }
        self.preview = Some(key);
    }
}

/// The inputs under the criteria box for the chosen criterion.
fn criterion_inputs(ui: &mut Ui, d: &mut Draft) {
    let boxed = |ui: &mut Ui, s: &mut String, hint: &str| {
        ui.add(egui::TextEdit::singleline(s).hint_text(hint).desired_width(PANEL_W - 8.0).margin(vec2(8.0, 8.0)));
    };
    match d.crit() {
        Crit::Dropdown => {
            let mut drop = None;
            for (i, it) in d.items.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    chip_colour(ui, i, &mut it.colour);
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut it.text)
                            .hint_text(format!("Option {}", i + 1))
                            .desired_width(PANEL_W - 84.0)
                            .margin(vec2(8.0, 6.0)),
                    );
                    crate::house::target(ui.ctx(), &format!("dv:item:{i}"), field.rect);
                    if let Some(img) = crate::icons::icon("delete")
                        && ui.add(egui::Button::image(img.fit_to_exact_size(vec2(18.0, 18.0))).frame(false)).on_hover_text("Delete item").clicked()
                    {
                        drop = Some(i);
                    }
                });
            }
            if let Some(i) = drop {
                d.items.remove(i);
            }
            let add = ui.add(crate::house::secondary("Add another item"));
            crate::house::target(ui.ctx(), "dv:add", add.rect);
            if add.clicked() {
                d.items.push(Item::default());
            }
            ui.add_space(4.0);
            let multi = ui.checkbox(&mut d.multiple, "Allow multiple selections");
            crate::house::target(ui.ctx(), "dv:multiple", multi.rect);
        }
        Crit::DropdownRange => {
            boxed(ui, &mut d.v1, "e.g. Sheet1!A1:A10");
            ui.add_space(4.0);
            let multi = ui.checkbox(&mut d.multiple, "Allow multiple selections");
            crate::house::target(ui.ctx(), "dv:multiple", multi.rect);
        }
        Crit::TextContains | Crit::TextNotContains | Crit::TextExactly => boxed(ui, &mut d.v1, "Value or formula"),
        Crit::Email | Crit::Url | Crit::ValidDate | Crit::TickBox => {}
        Crit::Date(op) => {
            let between = matches!(op, "between" | "notBetween");
            if !between {
                let labels: Vec<&str> = if op == "equal" { DATE_IS.iter().map(|x| x.0).collect() } else { DATE_REL.iter().map(|x| x.0).collect() };
                let mut when = d.when.min(labels.len() - 1);
                crate::house::select(ui, "dv_when", PANEL_W, &mut when, &labels, &[]);
                d.when = when;
            }
            if between || d.when == 0 {
                boxed(ui, &mut d.v1, "dd/mm/yyyy");
            }
            if between {
                boxed(ui, &mut d.v2, "dd/mm/yyyy");
            }
        }
        Crit::Number(op) => {
            boxed(ui, &mut d.v1, "Value or formula");
            if matches!(op, "between" | "notBetween") {
                boxed(ui, &mut d.v2, "Value or formula");
            }
        }
        Crit::Custom => boxed(ui, &mut d.v1, "=A1>0"),
    }
}

/// A drop-down item's colour, as in Sheets: a box with the colour dot and ▾, opening the light
/// and dark rows of chip colours.
fn chip_colour(ui: &mut Ui, i: usize, colour: &mut Option<u32>) {
    let (r, resp) = ui.allocate_exact_size(vec2(44.0, 30.0), Sense::click());
    let p = ui.painter();
    p.rect(r, 4.0, if resp.hovered() { theme::MENU_HOVER } else { Color32::WHITE }, Stroke::new(1.0, theme::BOX), egui::StrokeKind::Inside);
    let dot = r.left_center() + vec2(14.0, 0.0);
    p.circle(dot, 9.0, c32(colour.unwrap_or(CHIP_LIGHT[0])), Stroke::new(1.0, theme::BOX));
    let a = r.right_center() + vec2(-11.0, 0.0);
    p.add(egui::Shape::convex_polygon(vec![a + vec2(-4.0, -2.0), a + vec2(4.0, -2.0), a + vec2(0.0, 3.0)], theme::TEXT, Stroke::NONE));
    let resp = resp.on_hover_text("Colour");
    egui::Popup::menu(&resp).id(egui::Id::unique(("dv_chip", i))).show(|ui| {
        for row in [&CHIP_LIGHT, &CHIP_DARK] {
            ui.horizontal(|ui| {
                for c in row.iter() {
                    let pick = if *c == CHIP_LIGHT[0] { None } else { Some(*c) };
                    let (r, b) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
                    let ring = if *colour == pick {
                        Stroke::new(2.0, GREEN)
                    } else if b.hovered() {
                        Stroke::new(1.0, theme::TEXT_DIM)
                    } else {
                        Stroke::new(1.0, theme::BOX)
                    };
                    ui.painter().circle(r.center(), 10.0, c32(*c), ring);
                    if b.clicked() {
                        *colour = pick;
                        ui.close();
                    }
                }
            });
        }
    });
}

fn selection_text(app: &SheetApp) -> String {
    app.session.active().map(|d| d.selection.ranges.iter().map(|r| r.a1()).collect::<Vec<_>>().join(",")).unwrap_or_default()
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { format!("{}…", s.chars().take(n - 1).collect::<String>()) }
}

/// The top-left cell of a range list, written relatively (custom formulas are relative to it).
fn top_left(range: &str) -> Option<String> {
    crate::refs::top_left(range)
}

/// A formula string literal.
fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// A typed value: a formula (`=…`) is used as is, anything else as text.
fn text_arg(v: &str) -> String {
    match v.trim().strip_prefix('=') {
        Some(f) => f.to_string(),
        None => quoted(v.trim()),
    }
}

/// A date box: dd/mm/yyyy, or a formula.
fn date_arg(v: &str) -> Result<String, String> {
    let v = v.trim();
    if let Some(f) = v.strip_prefix('=') {
        return Ok(f.to_string());
    }
    date_number(v).map(|n| n.to_string()).ok_or_else(|| "Enter the date as dd/mm/yyyy.".to_string())
}

/// What the rule checks, for its card and the default help text.
fn describe(d: &Draft) -> String {
    let (label, crit, _) = CRITERIA[d.crit];
    match crit {
        Crit::Dropdown => format!("Drop-down: {}", d.items.iter().map(|i| i.text.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(", ")),
        Crit::DropdownRange => format!("Drop-down from {}", d.v1.trim()),
        Crit::Email | Crit::Url | Crit::ValidDate | Crit::TickBox => label.to_string(),
        Crit::Date(op) if !matches!(op, "between" | "notBetween") && d.when > 0 => {
            let when = if op == "equal" { DATE_IS[d.when.min(DATE_IS.len() - 1)].0 } else { DATE_REL[d.when.min(DATE_REL.len() - 1)].0 };
            format!("{label} {when}")
        }
        Crit::Date(op) | Crit::Number(op) if matches!(op, "between" | "notBetween") => format!("{label} {} and {}", d.v1.trim(), d.v2.trim()),
        Crit::Custom => format!("Custom formula is {}", d.v1.trim()),
        _ => format!("{label} {}", d.v1.trim()),
    }
}

/// Lower-cases only the first letter (keeps item names and values as typed).
fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_lowercase().chain(c).collect()).unwrap_or_default()
}

fn default_help(d: &Draft) -> String {
    match d.crit() {
        Crit::Dropdown | Crit::DropdownRange => "Choose an item from the list".to_string(),
        _ => format!("Enter a value: {}", lower_first(&describe(d))),
    }
}

/// The commands that put the draft on the sheet, in order.
fn commands(d: &Draft) -> Result<Vec<(&'static str, Json)>, String> {
    // Sheet names and `$` are allowed in the box (`Sheet1!$A$2:A`); the rule goes on this sheet.
    let range = crate::refs::plain(&d.range);
    let tl = top_left(&range).ok_or("Choose the cells the rule applies to (e.g. A1:A10).")?;
    let need = |v: &str| if v.trim().is_empty() { Err("Enter a value for the rule.".to_string()) } else { Ok(()) };
    let plain = d.display == PLAIN && matches!(d.crit(), Crit::Dropdown | Crit::DropdownRange);
    let mut p = json!({ "range": range, "errorStyle": if d.reject { "stop" } else { "warning" }, "dropdown": !plain });
    let mut extra = Vec::new();
    let custom = |f: String| json!({ "type": "custom", "formula1": format!("={f}") });
    let rule = match d.crit() {
        Crit::Dropdown => {
            let items: Vec<&Item> = d.items.iter().filter(|i| !i.text.trim().is_empty()).collect();
            if items.is_empty() {
                return Err("Add at least one item.".into());
            }
            if items.iter().any(|i| i.text.contains(',')) {
                return Err("An item can't contain a comma.".into());
            }
            for it in &items {
                if let Some(c) = it.colour {
                    let style = json!({ "fill": hex(c), "font": { "color": hex(chip_text(c)) } });
                    extra.push((
                        "home.conditionalFormat",
                        json!({ "range": range, "rule": { "type": "cellIs", "operator": "equal", "value": quoted(it.text.trim()), "style": style } }),
                    ));
                }
            }
            // The colours also go in the rule itself, for chips (and multiple selections, which an
            // "is equal to" rule can't colour); the conditional formats carry them to Excel.
            let colours: Vec<Json> = items.iter().filter_map(|i| i.colour.map(|c| json!({ "item": i.text.trim(), "color": hex(c) }))).collect();
            let formula1 = items.iter().map(|i| i.text.trim()).collect::<Vec<_>>().join(",");
            json!({ "type": "list", "formula1": formula1, "multiple": d.multiple, "display": DISPLAY[d.display].1, "itemColors": colours })
        }
        Crit::DropdownRange => {
            need(&d.v1)?;
            let formula1 = format!("={}", d.v1.trim().trim_start_matches('='));
            json!({ "type": "list", "formula1": formula1, "multiple": d.multiple, "display": DISPLAY[d.display].1 })
        }
        Crit::TextContains => {
            need(&d.v1)?;
            custom(format!("ISNUMBER(SEARCH({},{tl}))", text_arg(&d.v1)))
        }
        Crit::TextNotContains => {
            need(&d.v1)?;
            custom(format!("ISERROR(SEARCH({},{tl}))", text_arg(&d.v1)))
        }
        Crit::TextExactly => {
            need(&d.v1)?;
            custom(format!("EXACT({tl},{})", text_arg(&d.v1)))
        }
        Crit::Email => custom(format!("COUNTIF({tl},\"?*@?*.?*\")=1")),
        Crit::Url => custom(format!("OR(COUNTIF({tl},\"http://?*.?*\")=1,COUNTIF({tl},\"https://?*.?*\")=1)")),
        Crit::ValidDate => json!({ "type": "date", "operator": "between", "formula1": "1", "formula2": LAST_DATE }),
        Crit::Date(op) if matches!(op, "between" | "notBetween") => {
            json!({ "type": "date", "operator": op, "formula1": date_arg(&d.v1)?, "formula2": date_arg(&d.v2)? })
        }
        Crit::Date("equal") => {
            let (_, op, a, b) = DATE_IS[d.when.min(DATE_IS.len() - 1)];
            if a.is_empty() {
                json!({ "type": "date", "operator": "equal", "formula1": date_arg(&d.v1)? })
            } else if b.is_empty() {
                json!({ "type": "date", "operator": op, "formula1": a })
            } else {
                json!({ "type": "date", "operator": op, "formula1": a, "formula2": b })
            }
        }
        Crit::Date(op) => {
            let f = DATE_REL[d.when.min(DATE_REL.len() - 1)].1;
            let f = if f.is_empty() { date_arg(&d.v1)? } else { f.to_string() };
            json!({ "type": "date", "operator": op, "formula1": f })
        }
        Crit::Number(op) => {
            need(&d.v1)?;
            let mut r = json!({ "type": "decimal", "operator": op, "formula1": d.v1.trim() });
            if matches!(op, "between" | "notBetween") {
                need(&d.v2)?;
                r["formula2"] = json!(d.v2.trim());
            }
            r
        }
        Crit::TickBox => {
            extra.insert(0, ("insert.checkbox", json!({ "range": range })));
            custom(format!("ISLOGICAL({tl})"))
        }
        Crit::Custom => {
            need(&d.v1)?;
            custom(d.v1.trim().trim_start_matches('=').to_string())
        }
    };
    for (k, v) in rule.as_object().into_iter().flatten() {
        p[k] = v.clone();
    }
    if d.help_on {
        p["inputMessage"] = json!(if d.help.trim().is_empty() { default_help(d) } else { d.help.trim().to_string() });
    }
    p["errorTitle"] = json!("There was a problem");
    p["errorMessage"] = json!(format!("This cell's contents break its rule. Input must be: {}.", lower_first(&describe(d))));
    let mut out = vec![("data.validation", p)];
    out.extend(extra);
    Ok(out)
}

/// Delete on cells that hold nothing but have a drop-down or other rule takes the rule off them
/// (to-do 2 row 27). As in Google Sheets it is a second press: the first Delete clears what the
/// cells hold (GridCraft), a Delete on cells already empty removes their rule. A rule wholly
/// inside the selection goes with its item colours and its entry in StarSheets' section; one
/// only partly inside keeps its other cells.
pub fn delete_key(app: &mut SheetApp) {
    // Set by the grid's Delete when the selection held nothing (patch 0076).
    if !std::mem::take(&mut app.grid.delete_on_empty) {
        return;
    }
    let Some(d) = app.session.active() else { return };
    let Some(sh) = d.wb.active() else { return };
    let sel = d.selection.ranges.clone();
    let holds = sel.iter().any(|r| sh.cells.iter_range(*r).any(|(_, x)| !x.value.is_empty() || x.formula.is_some() || x.py.is_some()));
    let touched: Vec<Validation> = sh.validations.iter().filter(|v| v.ranges.iter().any(|r| sel.iter().any(|s| s.intersects(r)))).cloned().collect();
    if holds || touched.is_empty() {
        return;
    }
    let (cfs, theme_colours, sheet) = (sh.cond_formats.clone(), d.wb.theme.clone(), active_sheet_name(app));
    let mut h = crate::hostdata::read(app);
    let mut partly = false;
    for v in &touched {
        if v.ranges.iter().all(|r| sel.iter().any(|s| s.contains_range(r))) {
            let draft = draft_from(v, &cfs, &theme_colours, h.dv_for(&sheet, &crate::hostdata::ranges_text(&v.ranges)));
            remove(app, &orig_of(&draft), &mut h);
        } else {
            partly = true;
        }
    }
    if partly {
        let range = sel.iter().map(|r| r.a1()).collect::<Vec<_>>().join(",");
        app.run_or_alert("data.validation", json!({ "range": range, "clear": true }));
    }
    crate::hostdata::write(app, h);
}

/// Takes a rule off: its validation, the colours of its items (found by what they are, at the
/// moment of removing), its tick boxes and its entry in StarSheets' section (`h`, written by the
/// caller).
fn remove(app: &mut SheetApp, o: &Orig, h: &mut crate::hostdata::HostData) {
    if o.range.is_empty() {
        return;
    }
    let _ = app.run("data.validation", json!({ "range": o.range, "clear": true }));
    let idx: Vec<usize> = app
        .session
        .active()
        .and_then(|d| d.wb.active())
        .map(|sh| {
            sh.cond_formats
                .iter()
                .enumerate()
                .filter(|(_, cf)| crate::hostdata::ranges_text(&cf.ranges) == o.range && cf_item(cf).is_some_and(|v| o.coloured.contains(&v)))
                .map(|(i, _)| i)
                .collect()
        })
        .unwrap_or_default();
    for i in idx.into_iter().rev() {
        let _ = app.run("home.manageRules", json!({ "delete": i }));
    }
    if o.tick_box {
        let _ = app.run("home.numberFormat", json!({ "range": o.range, "format": "General" }));
    }
    h.remove_dv(&active_sheet_name(app), &o.range);
}

/// The item a conditional-formatting rule colours, if it is "is equal to" a piece of text.
fn cf_item(cf: &gridcraft_engine::model::CondFormat) -> Option<String> {
    let CfRule::CellIs { op: CfOperator::Equal, a, b: None, .. } = &cf.rule else { return None };
    let a = a.trim();
    (a.len() >= 2 && a.starts_with('"') && a.ends_with('"')).then(|| a[1..a.len() - 1].replace("\"\"", "\""))
}

/// What a draft puts on the sheet.
fn orig_of(d: &Draft) -> Orig {
    Orig {
        range: crate::hostdata::normalise_ranges(&d.range),
        coloured: if matches!(d.crit(), Crit::Dropdown) {
            d.items.iter().filter(|it| it.colour.is_some() && !it.text.trim().is_empty()).map(|it| it.text.trim().to_string()).collect()
        } else {
            Vec::new()
        },
        tick_box: matches!(d.crit(), Crit::TickBox),
    }
}

/// The section's entry for a draft.
fn meta_of(sheet: &str, d: &Draft, range: &str) -> crate::hostdata::DvMeta {
    let colours = if matches!(d.crit(), Crit::Dropdown) {
        d.items
            .iter()
            .filter(|it| !it.text.trim().is_empty())
            .filter_map(|it| it.colour.map(|c| crate::hostdata::ItemColour { text: it.text.trim().to_string(), colour: format!("#{c:06X}") }))
            .collect()
    } else {
        Vec::new()
    };
    crate::hostdata::DvMeta { sheet: sheet.to_string(), range: range.to_string(), crit: CRITERIA[d.crit].0.to_string(), colours }
}

fn active_sheet_name(app: &SheetApp) -> String {
    app.session.active().and_then(|d| d.wb.active()).map(|sh| sh.name.clone()).unwrap_or_default()
}

/// The text inside the first `"…"` of a formula (with `""` back to `"`), or the formula itself
/// with an `=` when the value was a formula.
fn first_quoted(f: &str, before: &str) -> String {
    let inner = &f[f.find('(').map_or(0, |i| i + 1)..];
    if let Some(rest) = inner.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = rest.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    out.push('"');
                } else {
                    return out;
                }
            } else {
                out.push(c);
            }
        }
        return out;
    }
    let arg = inner.split(before).next().unwrap_or(inner);
    format!("={arg}")
}

/// Reads a stored rule back into the editor's terms (the reverse of `commands`).
fn draft_from(
    dv: &Validation,
    cfs: &[gridcraft_engine::model::CondFormat],
    theme_colours: &gridcraft_engine::model::Theme,
    meta: Option<&crate::hostdata::DvMeta>,
) -> Draft {
    let range = dv.ranges.iter().map(|r| r.a1()).collect::<Vec<_>>().join(",");
    let mut d = Draft::new(range.clone());
    let tl = top_left(&range).unwrap_or_default();
    let pos = |c: Crit| CRITERIA.iter().position(|x| x.1 == c).unwrap_or(0);
    let op = op_name(dv.op);
    let f1 = dv.f1.trim().to_string();
    let f2 = dv.f2.clone().unwrap_or_default();
    let mut coloured = Vec::new();
    let mut tick_box = false;
    let show_date = |f: &str| f.parse::<i64>().ok().map(date_text).unwrap_or_else(|| format!("={f}"));
    match dv.kind {
        ValidationKind::List if f1.starts_with('"') => {
            d.crit = pos(Crit::Dropdown);
            let body = f1.trim_matches('"').replace("\"\"", "\"");
            d.items = body.split(',').map(|t| Item { text: t.trim().to_string(), colour: None }).collect();
            // With StarSheets' section only the items it lists have a colour rule; without it
            // (an older file) any "is equal to" rule on the same cells counts.
            let listed = |t: &str| meta.is_none_or(|m| m.colours.iter().any(|c| c.text == t));
            for cf in cfs {
                let same = crate::hostdata::ranges_text(&cf.ranges) == range;
                if let (true, CfRule::CellIs { op: CfOperator::Equal, a, b: None, style }) = (same, &cf.rule)
                    && let Some(it) = d.items.iter_mut().find(|it| quoted(&it.text) == *a && listed(it.text.trim()))
                {
                    it.colour = rgb_of(&style.fill.fg, theme_colours);
                    coloured.push(it.text.trim().to_string());
                }
            }
            // Colours kept in the rule itself (chips, multiple selections).
            for it in d.items.iter_mut().filter(|it| it.colour.is_none()) {
                it.colour = dv.item_color(it.text.trim()).and_then(|c| rgb_of(&c, theme_colours));
            }
        }
        ValidationKind::List => {
            d.crit = pos(Crit::DropdownRange);
            d.v1 = f1;
        }
        ValidationKind::Custom => {
            let f = f1.trim_start_matches('=');
            if f.starts_with("ISNUMBER(SEARCH(") {
                d.crit = pos(Crit::TextContains);
                d.v1 = first_quoted(&f["ISNUMBER(".len()..], ",");
            } else if f.starts_with("ISERROR(SEARCH(") {
                d.crit = pos(Crit::TextNotContains);
                d.v1 = first_quoted(&f["ISERROR(".len()..], ",");
            } else if let Some(rest) = f.strip_prefix("EXACT(") {
                d.crit = pos(Crit::TextExactly);
                let arg = rest.strip_prefix(&format!("{tl},")).unwrap_or(rest);
                d.v1 = first_quoted(&format!("({arg}"), ")");
                if d.v1.ends_with(')') && d.v1.starts_with('=') {
                    d.v1.pop();
                }
            } else if f.starts_with("COUNTIF(") && f.contains('@') {
                d.crit = pos(Crit::Email);
            } else if f.starts_with("OR(COUNTIF(") && f.contains("http") {
                d.crit = pos(Crit::Url);
            } else if f.starts_with("ISLOGICAL(") {
                d.crit = pos(Crit::TickBox);
                tick_box = true;
            } else {
                d.crit = pos(Crit::Custom);
                d.v1 = format!("={f}");
            }
        }
        ValidationKind::Date if op == "between" && f1 == "1" && f2 == LAST_DATE => d.crit = pos(Crit::ValidDate),
        ValidationKind::Date | ValidationKind::Time => {
            if let Some(w) = DATE_IS.iter().position(|x| x.1 == op && !x.2.is_empty() && x.2 == f1 && x.3 == f2) {
                d.crit = pos(Crit::Date("equal"));
                d.when = w;
            } else {
                let c = CRITERIA.iter().position(|x| x.1 == Crit::Date(static_op(op))).unwrap_or(0);
                d.crit = c;
                match DATE_REL.iter().position(|x| !x.1.is_empty() && x.1 == f1) {
                    Some(w) if !matches!(op, "between" | "notBetween") => d.when = w,
                    _ => {
                        d.v1 = show_date(&f1);
                        d.v2 = if f2.is_empty() { String::new() } else { show_date(&f2) };
                    }
                }
            }
        }
        _ => {
            d.crit = CRITERIA.iter().position(|x| x.1 == Crit::Number(static_op(op))).unwrap_or(0);
            d.v1 = f1;
            d.v2 = f2;
        }
    }
    d.help_on = !dv.input_message.is_empty();
    d.help = dv.input_message.clone();
    d.reject = dv.error_style == ErrorStyle::Stop;
    if dv.kind == ValidationKind::List {
        d.multiple = dv.multiple;
        d.display = match dv.display {
            _ if !dv.in_cell_dropdown => PLAIN,
            DropdownDisplay::Chip => 0,
            DropdownDisplay::Arrow => 1,
            DropdownDisplay::Plain => PLAIN,
        };
    }
    // StarSheets' section says when a custom formula only looks like one of the text criteria.
    if let (Some(m), ValidationKind::Custom) = (meta, dv.kind)
        && CRITERIA.iter().any(|c| c.0 == m.crit && c.1 == Crit::Custom)
        && d.crit() != Crit::Custom
    {
        d.crit = pos(Crit::Custom);
        d.v1 = format!("={}", dv.f1.trim().trim_start_matches('='));
        d.v2 = String::new();
        tick_box = false;
    }
    d.orig = Some(Orig { range, coloured, tick_box });
    d
}

fn op_name(op: CfOperator) -> &'static str {
    match op {
        CfOperator::Between => "between",
        CfOperator::NotBetween => "notBetween",
        CfOperator::Equal => "equal",
        CfOperator::NotEqual => "notEqual",
        CfOperator::Greater => "greater",
        CfOperator::Less => "less",
        CfOperator::GreaterOrEqual => "greaterOrEqual",
        CfOperator::LessOrEqual => "lessOrEqual",
    }
}

fn static_op(op: &str) -> &'static str {
    ["between", "notBetween", "equal", "notEqual", "greater", "less", "greaterOrEqual", "lessOrEqual"]
        .into_iter()
        .find(|x| *x == op)
        .unwrap_or("equal")
}

/// A spreadsheet date number as dd/mm/yyyy.
fn date_text(n: i64) -> String {
    // Civil from days (Howard Hinnant), from the 1899-12-30 epoch.
    let z = n - 25_569 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{day:02}/{month:02}/{year}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridcraft_engine::Session;
    use gridcraft_engine::core::CellRef;

    fn draft(crit: Crit, v1: &str, v2: &str) -> Draft {
        let mut d = Draft::new("B2:B9".into());
        d.crit = CRITERIA.iter().position(|x| x.1 == crit).unwrap();
        d.v1 = v1.into();
        d.v2 = v2.into();
        d
    }

    /// Puts a draft on a fresh sheet and checks typed input against it at B3.
    fn accepts(d: &Draft, input: &str) -> bool {
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        for (cmd, p) in commands(d).unwrap() {
            s.execute(cmd, p).unwrap();
        }
        let doc = s.active().unwrap();
        gridcraft_engine::cmd::data::check_validation(&doc.wb, 0, CellRef::new(2, 1), input).is_none()
    }

    #[test]
    fn the_fill_handle_carries_validation_on() {
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        let d = draft(Crit::TextContains, "cat", "");
        for (cmd, p) in commands(&d).unwrap() {
            s.execute(cmd, p).unwrap();
        }
        // The rule is on B2:B9; drag B9 down to B12.
        s.execute("edit.autoFill", json!({ "source": "B9", "target": "B9:B12" })).unwrap();
        let doc = s.active().unwrap();
        assert!(gridcraft_engine::cmd::data::check_validation(&doc.wb, 0, CellRef::new(11, 1), "dog").is_some());
        assert!(gridcraft_engine::cmd::data::check_validation(&doc.wb, 0, CellRef::new(11, 1), "cat").is_none());
    }

    #[test]
    fn criteria_check_input_like_sheets() {
        let d = draft(Crit::TextContains, "cat", "");
        assert!(accepts(&d, "Concatenate") && !accepts(&d, "dog"));
        let d = draft(Crit::TextNotContains, "cat", "");
        assert!(!accepts(&d, "a cat") && accepts(&d, "dog"));
        let d = draft(Crit::TextExactly, "Yes", "");
        assert!(accepts(&d, "Yes") && !accepts(&d, "yes"));
        let d = draft(Crit::Email, "", "");
        assert!(accepts(&d, "a@b.com") && !accepts(&d, "a@b") && !accepts(&d, "hello"));
        let d = draft(Crit::Url, "", "");
        assert!(accepts(&d, "https://x.org/a") && !accepts(&d, "x"));
        let d = draft(Crit::Number("between"), "1", "10");
        assert!(accepts(&d, "5") && !accepts(&d, "11"));
        let d = draft(Crit::Date("greater"), "01/01/2026", "");
        assert!(accepts(&d, "2026-02-01") && !accepts(&d, "2025-12-31"));
        let d = draft(Crit::ValidDate, "", "");
        assert!(accepts(&d, "2026-02-01") && !accepts(&d, "soon"));
        let mut d = draft(Crit::Dropdown, "", "");
        d.items = vec![Item { text: "Open".into(), colour: Some(0xD9EAD3) }, Item { text: "Closed".into(), colour: None }];
        assert!(accepts(&d, "Open") && !accepts(&d, "Maybe"));
    }

    #[test]
    fn rules_read_back_into_the_editor() {
        let mut cases = vec![
            draft(Crit::TextContains, "cat", ""),
            draft(Crit::TextNotContains, "=C1", ""),
            draft(Crit::TextExactly, "Say \"hi\"", ""),
            draft(Crit::Email, "", ""),
            draft(Crit::Url, "", ""),
            draft(Crit::ValidDate, "", ""),
            draft(Crit::Date("less"), "05/03/2026", ""),
            draft(Crit::Date("between"), "01/01/2026", "31/12/2026"),
            draft(Crit::Number("notBetween"), "1", "10"),
            draft(Crit::TickBox, "", ""),
            draft(Crit::Custom, "=B2>C2", ""),
        ];
        let mut rel = draft(Crit::Date("equal"), "", "");
        rel.when = 4;
        cases.push(rel);
        let mut dd = draft(Crit::Dropdown, "", "");
        dd.items = vec![Item { text: "Open".into(), colour: Some(0xD9EAD3) }, Item { text: "Closed".into(), colour: None }];
        dd.help_on = true;
        dd.help = "Pick one".into();
        dd.reject = false;
        cases.push(dd);
        for want in cases {
            let mut s = Session::new();
            s.execute("file.new", json!({})).unwrap();
            for (cmd, p) in commands(&want).unwrap() {
                s.execute(cmd, p).unwrap();
            }
            let doc = s.active().unwrap();
            let sh = doc.wb.active().unwrap().clone();
            let mut got = draft_from(&sh.validations[0], &sh.cond_formats, &doc.wb.theme, None);
            got.orig = None;
            assert!(got == want, "{} came back as {} ({:?} / {:?})", describe(&want), describe(&got), got.v1, got.v2);
        }
    }

    #[test]
    fn item_colours_are_told_apart_from_other_rules() {
        let mut d = draft(Crit::Dropdown, "", "");
        d.items = vec![Item { text: "Open".into(), colour: Some(0xD4EDBC) }, Item { text: "Closed".into(), colour: None }];
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        for (cmd, p) in commands(&d).unwrap() {
            s.execute(cmd, p).unwrap();
        }
        s.execute(
            "home.conditionalFormat",
            json!({ "range": "B2:B9", "rule": { "type": "cellIs", "operator": "equal", "value": "\"Maybe\"", "style": { "fill": "#FF0000" } } }),
        )
        .unwrap();
        let sh = s.active().unwrap().wb.active().unwrap().clone();
        let colour: Vec<bool> = sh.cond_formats.iter().map(|cf| is_item_colour(&sh, cf, &crate::hostdata::HostData::default())).collect();
        assert_eq!(colour.iter().filter(|c| **c).count(), 1, "{colour:?}");
        assert_eq!(colour.len(), 2);
    }

    #[test]
    fn the_section_settles_what_the_formulas_leave_open() {
        // A custom formula of the same shape as "Text contains" stays a custom formula.
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        let custom = draft(Crit::Custom, "=ISNUMBER(SEARCH(\"x\",B2))", "");
        for (cmd, p) in commands(&custom).unwrap() {
            s.execute(cmd, p).unwrap();
        }
        let sh = s.active().unwrap().wb.active().unwrap().clone();
        let theme = s.active().unwrap().wb.theme.clone();
        assert!(draft_from(&sh.validations[0], &[], &theme, None).crit() == Crit::TextContains, "without the section: guessed");
        let meta = meta_of(&sh.name, &custom, &crate::hostdata::normalise_ranges(&custom.range));
        let got = draft_from(&sh.validations[0], &[], &theme, Some(&meta));
        assert!(got.crit() == Crit::Custom && got.v1 == "=ISNUMBER(SEARCH(\"x\",B2))", "{:?}", got.v1);
        // A drop-down whose section lists no colours: an "is equal to" rule of the person's own
        // on the same cells stays theirs.
        let mut d = draft(Crit::Dropdown, "", "");
        d.items = vec![Item { text: "Open".into(), colour: None }, Item { text: "Closed".into(), colour: None }];
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        for (cmd, p) in commands(&d).unwrap() {
            s.execute(cmd, p).unwrap();
        }
        s.execute(
            "home.conditionalFormat",
            json!({ "range": "B2:B9", "rule": { "type": "cellIs", "operator": "equal", "value": "\"Open\"", "style": { "fill": "#FF0000" } } }),
        )
        .unwrap();
        let sh = s.active().unwrap().wb.active().unwrap().clone();
        let mut h = crate::hostdata::HostData::default();
        assert!(is_item_colour(&sh, &sh.cond_formats[0], &h), "without the section: guessed");
        h.set_dv(meta_of(&sh.name, &d, "B2:B9"));
        assert!(!is_item_colour(&sh, &sh.cond_formats[0], &h));
        assert!(draft_from(&sh.validations[0], &sh.cond_formats, &theme, h.dv_for(&sh.name, "B2:B9")).items[0].colour.is_none());
    }

    #[test]
    fn multiple_selection_chips_come_back_from_the_sheet_and_the_file() {
        let mut d = draft(Crit::Dropdown, "", "");
        d.items = vec![Item { text: "Open".into(), colour: Some(0xD4EDBC) }, Item { text: "Closed".into(), colour: None }];
        d.multiple = true;
        assert!(accepts(&d, "Open, Closed") && accepts(&d, "Closed") && !accepts(&d, "Open, Maybe"));
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        for (cmd, p) in commands(&d).unwrap() {
            s.execute(cmd, p).unwrap();
        }
        let wb = s.active().unwrap().wb.clone();
        let dv = &wb.active().unwrap().validations[0];
        assert!(dv.multiple && dv.display == DropdownDisplay::Chip);
        assert_eq!(dv.item_colors.len(), 1);
        // Through .xlsx (the settings Excel has no place for are kept beside the list rule).
        let bytes = gridcraft_engine::io::save_bytes(&wb, "t.xlsx").unwrap();
        let (back, _) = gridcraft_engine::io::open_bytes("t.xlsx", &bytes).unwrap();
        let sh = back.active().unwrap().clone();
        let mut got = draft_from(&sh.validations[0], &sh.cond_formats, &back.theme, None);
        got.orig = None;
        assert!(got == d, "came back as {} / multiple {} / display {}", describe(&got), got.multiple, got.display);
        // Each display style comes back.
        for style in 0..DISPLAY.len() {
            let mut d = d.clone();
            d.display = style;
            d.multiple = false;
            let mut s = Session::new();
            s.execute("file.new", json!({})).unwrap();
            for (cmd, p) in commands(&d).unwrap() {
                s.execute(cmd, p).unwrap();
            }
            let doc = s.active().unwrap();
            let sh = doc.wb.active().unwrap().clone();
            let got = draft_from(&sh.validations[0], &sh.cond_formats, &doc.wb.theme, None);
            assert_eq!(got.display, style, "{}", DISPLAY[style].0);
        }
    }

    #[test]
    fn a_colour_kept_only_in_the_rule_is_read_back() {
        // A multiple-selection cell holds "Open, Closed", which no "is equal to" rule colours:
        // the chip colour comes from the rule itself.
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        let colours = json!([{ "item": "Closed", "color": "#FFCFC9" }]);
        let p = json!({ "range": "B2:B9", "type": "list", "formula1": "Open,Closed", "multiple": true, "display": "chip", "itemColors": colours });
        s.execute("data.validation", p).unwrap();
        let doc = s.active().unwrap();
        let sh = doc.wb.active().unwrap().clone();
        let got = draft_from(&sh.validations[0], &sh.cond_formats, &doc.wb.theme, None);
        assert_eq!(got.items[1].colour, Some(0xFFCFC9));
        assert_eq!(got.items[0].colour, None);
        assert!(got.multiple);
    }

    #[test]
    fn dates_print_back() {
        assert_eq!(date_text(date_number("05/03/2026").unwrap()), "05/03/2026");
        assert_eq!(date_text(date_number("29/02/2024").unwrap()), "29/02/2024");
    }
}

/// Whether a conditional-formatting rule is a drop-down item's colour (it then belongs to the
/// data validation panel, not the conditional formatting one). With StarSheets' section: only a
/// rule it lists. Without (an older file): "is equal to" an item of a drop-down rule over exactly
/// the same cells.
pub fn is_item_colour(sh: &gridcraft_engine::model::Sheet, cf: &gridcraft_engine::model::CondFormat, h: &crate::hostdata::HostData) -> bool {
    let Some(value) = cf_item(cf) else { return false };
    if let Some(m) = h.dv_for(&sh.name, &crate::hostdata::ranges_text(&cf.ranges)) {
        return m.colours.iter().any(|c| c.text == value);
    }
    sh.validations.iter().any(|v| {
        v.kind == ValidationKind::List
            && v.ranges == cf.ranges
            && v.f1.trim().starts_with('"')
            && v.f1.trim().trim_matches('"').replace("\"\"", "\"").split(',').any(|item| item.trim() == value)
    })
}
