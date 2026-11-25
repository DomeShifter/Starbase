//! Edit › Find and replace (Ctrl+H), after Google Sheets: Find, Replace with, Search (all sheets,
//! this sheet or a range), Match case, Match entire cell contents, Search using regular
//! expressions, Also search within formulas; Find, Replace, Replace all and Done. Takes over
//! GridCraft's Find dialog. Ctrl+F opens Sheets' small find bar instead (top right): this sheet
//! only, "n of N", ↑ ↓ (or Enter / Shift+Enter) through the matches, ⋮ for the full Find and
//! replace, ✕ (or Esc) to close.

use egui::{RichText, vec2};
use gridcraft_engine::core::{CellRef, RangeRef, Value};
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

use crate::house;

const SCOPES: &[&str] = &["All sheets", "This sheet", "Specific range"];

#[derive(Default)]
pub struct FindReplace {
    open: bool,
    find: String,
    with: String,
    scope: usize,
    range: String,
    match_case: bool,
    whole: bool,
    regex: bool,
    formulas: bool,
    /// What happened last ("3 instances of x replaced", "No more results").
    status: String,
    focus: bool,
    /// The Ctrl+F bar: open, its text, focus wanted, the match shown and how many there are.
    bar: bool,
    bar_text: String,
    bar_focus: bool,
    bar_at: Option<usize>,
    bar_count: usize,
}

/// One search, compiled once.
struct Matcher {
    re: regex::Regex,
}

impl Matcher {
    fn new(f: &FindReplace) -> Result<Matcher, String> {
        let body = if f.regex { f.find.clone() } else { regex::escape(&f.find) };
        let body = if f.whole { format!("^(?:{body})$") } else { body };
        regex::RegexBuilder::new(&body)
            .case_insensitive(!f.match_case)
            .build()
            .map(|re| Matcher { re })
            .map_err(|_| "That isn't a valid regular expression.".to_string())
    }
}

impl FindReplace {
    /// Opens on Ctrl+H, Edit › Find and replace, and GridCraft's own Find dialog.
    pub fn intercept(&mut self, app: &mut SheetApp) {
        if app.dialog.as_ref().is_some_and(|d| matches!(d.name.as_str(), "find" | "replace")) {
            app.dialog = None;
            self.open = true;
            self.focus = true;
            self.status.clear();
        }
    }

    /// Ctrl+F and Ctrl+H (before the grid or the cell editor sees them, editing or not): the find
    /// bar and Find and replace. egui's text box would read Ctrl+H as Backspace, so the app takes it.
    /// Ctrl+W (delete the word before) is dropped too: no app uses it and a browser closes the tab.
    pub fn keys(&mut self, _app: &SheetApp, ctx: &egui::Context) {
        self.take_keys(ctx);
    }

    fn take_keys(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        let take = |ctx: &egui::Context, key| ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, key) | i.consume_key(Modifiers::CTRL, key));
        if take(ctx, Key::F) {
            self.bar = true;
            self.bar_focus = true;
        }
        if take(ctx, Key::H) {
            self.open = true;
            self.focus = true;
            self.status.clear();
        }
        take(ctx, Key::W);
    }

    /// This sheet's cells that show `bar_text` (any case), row by row.
    fn bar_matches(&self, app: &SheetApp) -> Vec<CellRef> {
        let Some(d) = app.session.active() else { return vec![] };
        let needle = self.bar_text.trim().to_lowercase();
        if needle.is_empty() {
            return vec![];
        }
        let sheet = d.wb.active_sheet();
        let Some(sh) = d.wb.sheets.get(sheet) else { return vec![] };
        let mut cells: Vec<CellRef> = sh
            .cells
            .iter()
            .filter(|(_, c)| !matches!(c.value, Value::Empty))
            .map(|(at, c)| (at, gridcraft_engine::display::format(&c.value, d.wb.styles.get(c.style).num_fmt.as_str(), &d.wb).text))
            .filter(|(_, t)| t.to_lowercase().contains(&needle))
            .map(|(at, _)| at)
            .collect();
        cells.sort_by_key(|c| (c.row, c.col));
        cells
    }

    /// The Ctrl+F bar, top right of the sheet.
    pub fn bar_ui(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        if !self.bar {
            return;
        }
        let (mut go, mut close, mut full) = (None::<i32>, false, false);
        egui::Area::new(egui::Id::unique("starsheets_find_bar"))
            .anchor(egui::Align2::RIGHT_TOP, vec2(-24.0, 136.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                house::card_frame().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut self.bar_text)
                                .desired_width(200.0)
                                .margin(vec2(house::M, house::S + 1.0))
                                .hint_text("Find in sheet"),
                        );
                        if self.bar_focus {
                            r.request_focus();
                            self.bar_focus = false;
                        }
                        if r.changed() {
                            self.bar_at = None;
                            go = Some(0);
                        }
                        let (enter, shift, esc) =
                            ui.input(|i| (i.key_pressed(egui::Key::Enter), i.modifiers.shift, i.key_pressed(egui::Key::Escape)));
                        if r.lost_focus() && enter {
                            go = Some(if shift { -1 } else { 1 });
                            self.bar_focus = true;
                        }
                        if esc && (r.has_focus() || r.lost_focus()) {
                            close = true;
                        }
                        let shown = match (self.bar_at, self.bar_count) {
                            (_, 0) => "0 of 0".to_string(),
                            (Some(i), n) => format!("{} of {n}", i + 1),
                            (None, n) => format!("{n} found"),
                        };
                        ui.label(RichText::new(shown).size(house::BODY).color(crate::theme::TEXT_DIM));
                        let icon = |ui: &mut egui::Ui, name: &str, fallback: &str, tip: &str| match crate::icons::icon(name) {
                            Some(img) => ui.add(egui::Button::image(img).frame(false)).on_hover_text(tip),
                            None => ui.add(egui::Button::new(fallback).frame(false)).on_hover_text(tip),
                        };
                        if icon(ui, "keyboard_arrow_up", "▲", "Previous (Shift+Enter)").clicked() {
                            go = Some(-1);
                        }
                        if icon(ui, "keyboard_arrow_down", "▼", "Next (Enter)").clicked() {
                            go = Some(1);
                        }
                        if ui.add(egui::Button::new(RichText::new("⋮").size(16.0)).frame(false)).on_hover_text("Find and replace (Ctrl+H)").clicked()
                        {
                            full = true;
                        }
                        if icon(ui, "close", "✕", "Close (Esc)").clicked() {
                            close = true;
                        }
                    });
                });
            });
        if let Some(step) = go {
            let all = self.bar_matches(app);
            self.bar_count = all.len();
            if all.is_empty() {
                self.bar_at = None;
            } else {
                let n = all.len();
                let i = match (self.bar_at, step) {
                    (Some(i), 1) => (i + 1) % n,
                    (Some(i), -1) => (i + n - 1) % n,
                    (Some(i), _) => i.min(n - 1),
                    (None, -1) => n - 1,
                    (None, _) => 0,
                };
                self.bar_at = Some(i);
                let sheet = app.session.active().map_or(0, |d| d.wb.active_sheet());
                Self::select(app, sheet, all[i]);
            }
        }
        if full {
            self.find = self.bar_text.clone();
            self.open = true;
            self.focus = true;
            self.status.clear();
            close = true;
        }
        if close {
            self.bar = false;
            self.bar_at = None;
        }
    }

    pub fn ui(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let mut closed = false;
        let mut action = None;
        house::dialog(ctx, "starsheets_find_replace", 460.0, "Find and replace", Some(&mut closed), |ui| {
            let field = |ui: &mut egui::Ui, label: &str, text: &mut String, hint: &str| {
                ui.horizontal(|ui| {
                    label_cell(ui, label);
                    ui.add(
                        egui::TextEdit::singleline(text)
                            .desired_width(460.0 - 110.0 - house::S)
                            .margin(vec2(house::M, house::S + 1.0))
                            .hint_text(hint),
                    )
                })
                .inner
            };
            let r = field(ui, "Find", &mut self.find, "");
            if self.focus {
                r.request_focus();
                self.focus = false;
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                action = Some("find");
            }
            field(ui, "Replace with", &mut self.with, "");
            ui.horizontal(|ui| {
                label_cell(ui, "Search");
                house::select(ui, "find_scope", 460.0 - 110.0 - house::S, &mut self.scope, SCOPES, &[]);
            });
            if self.scope == 2 {
                field(ui, "", &mut self.range, "e.g. Sheet1!A1:D20");
            }
            ui.add_space(house::XS);
            ui.checkbox(&mut self.match_case, "Match case");
            ui.checkbox(&mut self.whole, "Match entire cell contents");
            ui.checkbox(&mut self.regex, "Search using regular expressions");
            ui.checkbox(&mut self.formulas, "Also search within formulas");
            if !self.status.is_empty() {
                house::note(ui, &self.status);
            }
            ui.add_space(house::XL - house::S);
            ui.horizontal(|ui| {
                for (label, what) in [("Find", "find"), ("Replace", "replace"), ("Replace all", "all")] {
                    if ui.add(house::secondary(label).min_size(vec2(0.0, house::BUTTON_HEIGHT))).clicked() {
                        action = Some(what);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(house::primary("Done")).clicked() {
                        action = Some("done");
                    }
                });
            });
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                action = Some("done");
            }
        });
        if closed {
            action = Some("done");
        }
        match action {
            Some("done") => self.open = false,
            Some(a) if self.find.is_empty() => {
                let _ = a;
                self.status = "Type something to find.".into();
            }
            Some("find") => self.find_next(app),
            Some("replace") => self.replace_one(app),
            Some("all") => self.replace_all(app),
            _ => {}
        }
    }

    /// The (sheet, range) pairs to search, in order, starting with the active sheet.
    fn places(&self, app: &SheetApp) -> Vec<(usize, Option<RangeRef>)> {
        let Some(d) = app.session.active() else { return vec![] };
        let active = d.wb.active_sheet();
        match self.scope {
            1 => vec![(active, None)],
            2 => {
                let (sheet, body) = match self.range.rsplit_once('!') {
                    Some((s, b)) => (d.wb.sheet_index(s.trim_matches('\'')).unwrap_or(active), b),
                    None => (active, self.range.as_str()),
                };
                RangeRef::parse(&body.replace('$', "")).map(|r| vec![(sheet, Some(r))]).unwrap_or_default()
            }
            _ => (0..d.wb.sheets.len()).map(|i| ((active + i) % d.wb.sheets.len(), None)).collect(),
        }
    }

    /// The text a cell is searched in: what it shows, or its formula when that's asked for.
    fn text_of(&self, app: &SheetApp, sheet: usize, c: CellRef) -> Option<(String, bool)> {
        let d = app.session.active()?;
        let sh = d.wb.sheets.get(sheet)?;
        let cell = sh.cell(c)?;
        if cell.formula.is_some() {
            return self.formulas.then(|| (sh.input_text(c), true));
        }
        if matches!(cell.value, Value::Empty) {
            return None;
        }
        let code = d.wb.styles.get(cell.style).num_fmt.as_str().to_string();
        Some((gridcraft_engine::display::format(&cell.value, &code, &d.wb).text, false))
    }

    /// Every matching cell, in search order (row by row).
    fn matches(&mut self, app: &SheetApp) -> Vec<(usize, CellRef)> {
        let m = match Matcher::new(self) {
            Ok(m) => m,
            Err(e) => {
                self.status = e;
                return vec![];
            }
        };
        let Some(d) = app.session.active() else { return vec![] };
        let mut out = vec![];
        for (sheet, within) in self.places(app) {
            let Some(sh) = d.wb.sheets.get(sheet) else { continue };
            let mut cells: Vec<CellRef> = sh.cells.iter().map(|(c, _)| c).filter(|c| within.is_none_or(|r| r.contains(*c))).collect();
            cells.sort_by_key(|c| (c.row, c.col));
            for c in cells {
                if let Some((t, _)) = self.text_of(app, sheet, c)
                    && m.re.is_match(&t)
                {
                    out.push((sheet, c));
                }
            }
        }
        out
    }

    fn select(app: &mut SheetApp, sheet: usize, c: CellRef) {
        let name = app.session.active().and_then(|d| d.wb.sheets.get(sheet).map(|s| s.name.clone())).unwrap_or_default();
        if app.session.active().is_some_and(|d| d.wb.active_sheet() != sheet) {
            app.run_or_alert("sheet.activate", json!({ "sheet": sheet }));
        }
        if app.run("edit.goTo", json!({ "reference": format!("'{}'!{}", name.replace('\'', "''"), c.a1()) })).is_ok() {
            // Scroll the match into view, as the Name Box and Go To do (to-do 2 row 31).
            app.grid.ensure_visible = true;
        }
    }

    /// Moves to the next match after the active cell (wrapping round).
    fn find_next(&mut self, app: &mut SheetApp) {
        let all = self.matches(app);
        if all.is_empty() {
            if self.status.is_empty() || !self.status.starts_with("That") {
                self.status = format!("No results for \"{}\".", self.find);
            }
            return;
        }
        let here = app.session.active().map(|d| (d.wb.active_sheet(), d.selection.active));
        // The next match below/right of the active cell on this sheet, else the first on the
        // following sheets, else back to the first.
        let next = here
            .and_then(|(s, c)| all.iter().position(|(ms, mc)| *ms == s && (mc.row, mc.col) > (c.row, c.col)))
            .or_else(|| here.and_then(|(s, _)| all.iter().position(|(ms, _)| *ms != s)))
            .unwrap_or(0);
        let (s, c) = all[next];
        self.status = format!("Result {} of {}", next + 1, all.len());
        Self::select(app, s, c);
    }

    /// Replaces in the active cell when it matches, then moves to the next match.
    fn replace_one(&mut self, app: &mut SheetApp) {
        let Ok(m) = Matcher::new(self) else {
            self.status = "That isn't a valid regular expression.".into();
            return;
        };
        if let Some((s, c)) = app.session.active().map(|d| (d.wb.active_sheet(), d.selection.active))
            && let Some((t, formula)) = self.text_of(app, s, c)
            && m.re.is_match(&t)
        {
            let new = self.replaced(&m, &t);
            let input = if formula && !new.starts_with('=') { format!("={new}") } else { new };
            app.run_or_alert("cell.set", json!({ "cell": c.a1(), "input": input }));
        }
        self.find_next(app);
    }

    fn replaced(&self, m: &Matcher, t: &str) -> String {
        if self.regex { m.re.replace_all(t, self.with.as_str()).into_owned() } else { m.re.replace_all(t, regex::NoExpand(&self.with)).into_owned() }
    }

    /// Replaces every match in the search area.
    fn replace_all(&mut self, app: &mut SheetApp) {
        let all = self.matches(app);
        let Ok(m) = Matcher::new(self) else { return };
        if all.is_empty() {
            self.status = format!("No results for \"{}\".", self.find);
            return;
        }
        let back = app.session.active().map(|d| d.wb.active_sheet());
        let mut done = 0;
        for (s, c) in &all {
            let Some((t, formula)) = self.text_of(app, *s, *c) else { continue };
            let new = self.replaced(&m, &t);
            let input = if formula && !new.starts_with('=') { format!("={new}") } else { new };
            let name = app.session.active().and_then(|d| d.wb.sheets.get(*s).map(|x| x.name.clone())).unwrap_or_default();
            if app.run("cell.set", json!({ "cell": c.a1(), "input": input, "sheet": name })).is_ok() {
                done += 1;
            }
        }
        if let Some(b) = back
            && app.session.active().is_some_and(|d| d.wb.active_sheet() != b)
        {
            app.run_or_alert("sheet.activate", json!({ "sheet": b }));
        }
        self.status = format!("Replaced {done} instance{} of \"{}\" with \"{}\".", if done == 1 { "" } else { "s" }, self.find, self.with);
    }
}

/// A field's label, left-aligned in a fixed 110 px column.
fn label_cell(ui: &mut egui::Ui, label: &str) {
    ui.allocate_ui_with_layout(vec2(110.0, house::FIELD_HEIGHT), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_width(110.0);
        ui.label(RichText::new(label).size(house::BODY));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ctrl+H/Ctrl+F/Ctrl+W are taken by the app, whether or not a cell is being edited; the text
    /// box never sees them (it would delete a character or a word).
    #[test]
    fn find_keys_are_consumed_before_the_text_box() {
        use egui::{Event, Key, Modifiers};
        let press = |key| {
            let ctx = egui::Context::default();
            let mut f = FindReplace::default();
            let mut left = true;
            let mut raw = egui::RawInput::default();
            raw.events.push(Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: Modifiers::CTRL });
            ctx.begin_pass(raw);
            f.take_keys(&ctx);
            left = ctx.input(|i| i.events.iter().any(|e| matches!(e, Event::Key { key: k, .. } if *k == key)));
            (f.open, f.bar, left)
        };
        assert_eq!(press(Key::H), (true, false, false));
        assert_eq!(press(Key::F), (false, true, false));
        assert_eq!(press(Key::W), (false, false, false));
        assert_eq!(press(Key::K), (false, false, true), "Ctrl+K is left alone");
    }

    #[test]
    fn matching_follows_the_options() {
        let mut f = FindReplace { find: "cat".into(), ..Default::default() };
        assert!(Matcher::new(&f).unwrap().re.is_match("Concatenate"));
        f.match_case = true;
        assert!(!Matcher::new(&f).unwrap().re.is_match("CAT"));
        f.whole = true;
        assert!(!Matcher::new(&f).unwrap().re.is_match("cats"));
        f = FindReplace { find: "a.c".into(), ..Default::default() };
        assert!(!Matcher::new(&f).unwrap().re.is_match("abc"), "without regex a dot is a dot");
        f.regex = true;
        assert!(Matcher::new(&f).unwrap().re.is_match("abc"));
        f.with = "[$0]".into();
        assert_eq!(f.replaced(&Matcher::new(&f).unwrap(), "xabcx"), "x[abc]x");
        f.regex = false;
        f.find = "x".into();
        assert_eq!(f.replaced(&Matcher::new(&f).unwrap(), "xx"), "[$0][$0]", "without regex $ is literal");
    }
}
