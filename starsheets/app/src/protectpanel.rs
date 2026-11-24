//! Data › Protect sheets and ranges, after Google Sheets: a side panel listing what is protected,
//! "Add a sheet or range" (a description, a range or a whole sheet with "Except certain cells"),
//! and permissions: show a warning when editing, or restrict editing.
//!
//! How it maps onto the spreadsheet (and so onto Excel files):
//! - each entry is a hidden name `_sc_protect<n>` holding the range (so it follows inserted rows
//!   and columns); its description and settings are in StarSheets' section of the file
//!   (hostdata.rs), and the name's comment holds the description in plain words (older files
//!   kept the settings there as JSON, still read when the section has no entry);
//! - "Restrict editing" protects the sheet for real. A protected range is everything locked
//!   except the rest of the sheet, which becomes one allowed edit range; a protected sheet keeps
//!   its "except" cells as that allowed range;
//! - "Show a warning" asks before an edit starts in the range (Google's "Heads up!").

use egui::{RichText, Stroke, Ui, vec2};
use gridcraft_engine::core::{CellRef, RangeRef};
use gridcraft_ui_egui::SheetApp;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{house, theme};

/// Asks for the panel; the value is a sheet index to start "Protect sheet" on, or -1 for the list.
pub const REQUEST: &str = "starsheets_protect";
const PANEL_W: f32 = 320.0;
const PREFIX: &str = "_sc_protect";
/// The allowed edit range StarSheets keeps on a protected sheet.
const FREE_TITLE: &str = "StarSheets editable cells";

/// What an entry's name comment holds.
#[derive(Clone, Default, Serialize, Deserialize)]
struct Meta {
    #[serde(default)]
    d: String,
    #[serde(default)]
    sheet: bool,
    #[serde(default)]
    warn: bool,
    #[serde(default)]
    except: String,
}

/// One protected sheet or range.
#[derive(Clone)]
struct Entry {
    name: String,
    sheet: usize,
    range: RangeRef,
    meta: Meta,
}

struct Draft {
    old: Option<String>,
    desc: String,
    sheet_mode: bool,
    range: String,
    sheet: usize,
    except_on: bool,
    except: String,
    warn: bool,
}

#[derive(Default)]
pub struct ProtectPanel {
    pub open: bool,
    pub just_opened: bool,
    draft: Option<Draft>,
    /// A warned edit waiting for "OK", and the time the warning may show again.
    asking: Option<CellRef>,
    /// The warned cell the current edit was allowed on (asked once per edit).
    allowed: Option<CellRef>,
    quiet_until: f64,
    dont_ask: bool,
    /// OK was pressed: start the edit on the next frame.
    start_edit: bool,
    /// What the edit held when the warning stopped it (the key that was typed, or the cell's text
    /// for F2 / double-click), so OK carries on from there and nothing typed is lost.
    held_text: Option<String>,
}

fn entries(app: &SheetApp) -> Vec<Entry> {
    let Some(d) = app.session.active() else { return vec![] };
    let h = d.wb.host_data.as_deref().map(crate::hostdata::parse).unwrap_or_default();
    d.wb.names
        .iter()
        .filter(|n| n.name.starts_with(PREFIX))
        .filter_map(|n| {
            let (sheet, body) = n.formula.trim_start_matches('=').rsplit_once('!')?;
            let si = d.wb.sheet_index(sheet.trim_matches('\'').replace("''", "'").as_str())?;
            let range = RangeRef::parse(&body.replace('$', ""))?;
            let meta = h
                .protect_for(&n.name)
                .and_then(|m| serde_json::from_value(m.clone()).ok())
                .or_else(|| serde_json::from_str(&n.comment).ok())
                .unwrap_or_default();
            Some(Entry { name: n.name.clone(), sheet: si, range, meta })
        })
        .collect()
}

fn quoted(name: &str) -> String {
    if name.chars().all(|c| c.is_alphanumeric() || c == '_') { name.to_string() } else { format!("'{}'", name.replace('\'', "''")) }
}

fn sheet_name(app: &SheetApp, i: usize) -> String {
    app.session.active().and_then(|d| d.wb.sheets.get(i).map(|s| s.name.clone())).unwrap_or_default()
}

/// The cells of the sheet not in `taken`, as a few rectangles (rows split where a range starts
/// or ends, then the free columns of each band).
fn complement(taken: &[RangeRef]) -> Vec<RangeRef> {
    use gridcraft_engine::core::{MAX_COLS, MAX_ROWS};
    let mut cuts: Vec<u32> = vec![0, MAX_ROWS];
    for r in taken {
        cuts.push(r.start.row);
        cuts.push(r.end.row + 1);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut out = vec![];
    for w in cuts.windows(2) {
        let (r0, r1) = (w[0], w[1]);
        if r0 >= r1 || r0 >= MAX_ROWS {
            continue;
        }
        let mut cols: Vec<(u32, u32)> = taken.iter().filter(|r| r.start.row <= r0 && r.end.row >= r1 - 1).map(|r| (r.start.col, r.end.col)).collect();
        cols.sort_unstable();
        let mut c = 0;
        for (a, b) in cols {
            if a > c {
                out.push(RangeRef::new(CellRef::new(r0, c), CellRef::new(r1 - 1, a - 1)));
            }
            c = c.max(b + 1);
        }
        if c < MAX_COLS {
            out.push(RangeRef::new(CellRef::new(r0, c), CellRef::new(r1 - 1, MAX_COLS - 1)));
        }
    }
    out
}

impl ProtectPanel {
    pub fn close(&mut self) {
        self.open = false;
        self.draft = None;
    }

    pub fn intercept(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        self.just_opened = false;
        let mut want = ctx.data_mut(|m| m.remove_temp::<i64>(egui::Id::unique(REQUEST)));
        // GridCraft's own Protect Sheet / Allow Edit Ranges dialogs open the panel instead.
        if app.dialog.as_ref().is_some_and(|d| matches!(d.name.as_str(), "protectSheet" | "allowEditRanges")) {
            app.dialog = None;
            want = Some(-1);
        }
        if let Some(w) = want {
            self.open = true;
            self.just_opened = true;
            self.draft = None;
            if w >= 0 {
                self.new_draft(app, Some(w as usize));
            }
        }
    }

    /// The "Heads up!" warning for warned ranges: once per frame, after the grid (web.rs). It was
    /// drawn from `intercept`, which runs twice a frame, so each click on its checkbox toggled it
    /// twice (it never ticked) and OK could be handled before the grid saw the same click.
    pub fn warning_ui(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        // The edit allowed last frame starts now, after that click has gone.
        if std::mem::take(&mut self.start_edit) {
            app.begin_edit(self.held_text.take(), false);
        }
        self.warn(app, ctx);
    }

    fn new_draft(&mut self, app: &SheetApp, sheet: Option<usize>) {
        let Some(d) = app.session.active() else { return };
        let r = d.selection.ranges.first().copied().unwrap_or_else(|| RangeRef::new(d.selection.active, d.selection.active));
        let active = d.wb.active_sheet();
        self.draft = Some(Draft {
            old: None,
            desc: String::new(),
            sheet_mode: sheet.is_some(),
            range: format!("{}!{}", quoted(&sheet_name(app, active)), r.a1()),
            sheet: sheet.unwrap_or(active),
            except_on: false,
            except: String::new(),
            warn: false,
        });
    }

    pub fn ui(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        if !self.open {
            return;
        }
        egui::Panel::right("starsheets_protect").exact_size(PANEL_W + 32.0 + house::SCROLL_ROOM).resizable(false).frame(house::panel_frame()).show(
            ui,
            |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Protected sheets and ranges").size(16.0).color(theme::TEXT));
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
        if ui.add(house::link("+ Add a sheet or range")).clicked() {
            self.new_draft(app, None);
            return;
        }
        ui.add_space(house::S);
        let list = entries(app);
        if list.is_empty() {
            house::note(
                ui,
                "Nothing is protected. Protecting a range or sheet stops it being changed by accident: edits are refused, or you are warned first.",
            );
        }
        for e in list {
            let what = if e.meta.sheet { sheet_name(app, e.sheet) } else { format!("{}!{}", sheet_name(app, e.sheet), e.range.a1()) };
            let title = if e.meta.d.is_empty() { what.clone() } else { e.meta.d.clone() };
            let how = if e.meta.warn { "Warning" } else { "Restricted" };
            let r = ui.add(
                egui::Button::new(RichText::new(&title).size(house::BODY).strong().color(theme::TEXT))
                    .right_text(RichText::new(format!("{what} · {how}")).size(house::NOTE).color(theme::TEXT_DIM))
                    .frame_when_inactive(false)
                    .min_size(vec2(PANEL_W, 44.0)),
            );
            ui.painter().hline(r.rect.x_range(), r.rect.bottom(), Stroke::new(1.0, theme::DIVIDER));
            if r.clicked() {
                self.draft = Some(Draft {
                    old: Some(e.name.clone()),
                    desc: e.meta.d.clone(),
                    sheet_mode: e.meta.sheet,
                    range: format!("{}!{}", quoted(&sheet_name(app, e.sheet)), e.range.a1()),
                    sheet: e.sheet,
                    except_on: !e.meta.except.is_empty(),
                    except: e.meta.except.clone(),
                    warn: e.meta.warn,
                });
            }
        }
    }

    fn editor(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        let sheets: Vec<String> = app.session.active().map(|d| d.wb.sheets.iter().map(|s| s.name.clone()).collect()).unwrap_or_default();
        let Some(d) = self.draft.as_mut() else { return };
        house::label(ui, "Description", "");
        ui.add(
            egui::TextEdit::singleline(&mut d.desc).desired_width(PANEL_W).margin(vec2(house::M, house::S + 1.0)).hint_text("Enter a description"),
        );
        house::gap(ui);
        let mut mode = usize::from(d.sheet_mode);
        ui.horizontal(|ui| {
            for (i, label) in ["Range", "Sheet"].iter().enumerate() {
                let on = mode == i;
                let r = ui.add(
                    egui::Button::new(RichText::new(*label).size(house::BODY).color(if on { house::GREEN } else { theme::TEXT_DIM }))
                        .frame(false)
                        .min_size(vec2(PANEL_W / 2.0 - 4.0, 36.0)),
                );
                if on {
                    ui.painter().hline(r.rect.x_range().shrink(8.0), r.rect.bottom(), Stroke::new(3.0, house::GREEN));
                }
                if r.clicked() {
                    mode = i;
                }
            }
        });
        d.sheet_mode = mode == 1;
        ui.add_space(house::S);
        if d.sheet_mode {
            let labels: Vec<&str> = sheets.iter().map(String::as_str).collect();
            if !labels.is_empty() {
                d.sheet = d.sheet.min(labels.len() - 1);
                house::select(ui, "protect_sheet", PANEL_W, &mut d.sheet, &labels, &[]);
            }
            ui.checkbox(&mut d.except_on, "Except certain cells");
            if d.except_on {
                ui.add(
                    egui::TextEdit::singleline(&mut d.except)
                        .desired_width(PANEL_W)
                        .margin(vec2(house::M, house::S + 1.0))
                        .hint_text("e.g. A1:B10, D2:D20"),
                );
            }
        } else {
            ui.add(
                egui::TextEdit::singleline(&mut d.range)
                    .desired_width(PANEL_W)
                    .margin(vec2(house::M, house::S + 1.0))
                    .hint_text("e.g. Sheet1!A1:D10"),
            );
        }
        house::gap(ui);
        house::label(ui, "When someone edits it", "");
        let mut warn = usize::from(d.warn);
        ui.radio_value(&mut warn, 1, "Show a warning, then allow the edit");
        ui.radio_value(&mut warn, 0, "Restrict editing (only you, by removing the protection here)");
        d.warn = warn == 1;
        let editing = d.old.is_some();
        let labels: &[&str] = if editing { &["Remove", "Cancel", "Done"] } else { &["Cancel", "Done"] };
        let chosen = house::actions(ui, labels).map(|i| labels[i]);
        match chosen {
            Some("Cancel") => self.draft = None,
            Some("Remove") => {
                if let Some(old) = d.old.clone() {
                    let sheet = d.sheet;
                    app.run_or_alert("formulas.deleteName", json!({ "name": old, "force": true }));
                    // Drops the entry's settings from the section (its name is gone).
                    let h = crate::hostdata::read(app);
                    crate::hostdata::write(app, h);
                    self.draft = None;
                    sync(app, sheet);
                }
            }
            Some("Done") => {
                let Some(d) = self.draft.take() else { return };
                let (sheet, refers) = if d.sheet_mode {
                    (d.sheet, format!("={}!$A$1:$XFD$1048576", quoted(sheets.get(d.sheet).map_or("", String::as_str))))
                } else {
                    let si = d
                        .range
                        .rsplit_once('!')
                        .and_then(|(s, _)| app.session.active().and_then(|x| x.wb.sheet_index(s.trim_matches('\''))))
                        .unwrap_or_else(|| app.session.active().map_or(0, |x| x.wb.active_sheet()));
                    let body = d.range.rsplit_once('!').map_or(d.range.as_str(), |(_, b)| b);
                    (si, format!("={}!{}", quoted(sheets.get(si).map_or("", String::as_str)), crate::namespanel::absolute(body.trim())))
                };
                let meta = Meta {
                    d: d.desc.trim().to_string(),
                    sheet: d.sheet_mode,
                    warn: d.warn,
                    except: if d.except_on { d.except.trim().to_string() } else { String::new() },
                };
                let name = d.old.clone().unwrap_or_else(|| {
                    let have: Vec<String> = entries(app).into_iter().map(|e| e.name).collect();
                    (1..).map(|i| format!("{PREFIX}{i}")).find(|n| !have.contains(n)).unwrap_or_default()
                });
                let mut p = json!({ "name": name, "refersTo": refers, "comment": meta.d, "force": true });
                if let Some(old) = &d.old {
                    p["oldName"] = json!(old);
                }
                let was_on = d.old.as_ref().and_then(|o| entries(app).into_iter().find(|e| &e.name == o)).map(|e| e.sheet);
                match app.run("formulas.defineName", p) {
                    Ok(_) => {
                        let mut h = crate::hostdata::read(app);
                        h.set_protect(&name, serde_json::to_value(&meta).unwrap_or_default());
                        crate::hostdata::write(app, h);
                        sync(app, sheet);
                        // An entry moved to another sheet: that sheet's protection changes too.
                        if let Some(w) = was_on.filter(|w| *w != sheet) {
                            sync(app, w);
                        }
                    }
                    Err(e) => {
                        app.message = Some(("Protect sheets and ranges".into(), e.to_string()));
                        self.draft = Some(d);
                    }
                }
            }
            _ => {}
        }
    }

    /// Google's "Heads up!" before an edit in a warned range.
    fn warn(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let editing = app.editor.as_ref().map(|e| e.cell);
        if editing.is_none() && !self.start_edit {
            self.allowed = None;
        }
        if self.asking.is_none()
            && now >= self.quiet_until
            && editing != self.allowed
            && let Some(cell) = app.editor.as_ref().map(|e| e.cell)
            && let Some(sheet) = app.session.active().map(|d| d.wb.active_sheet())
            && entries(app).iter().any(|e| e.meta.warn && e.sheet == sheet && e.range.contains(cell) && !except_contains(&e.meta.except, cell))
        {
            self.held_text = app.editor.as_ref().map(|e| e.text.clone());
            app.cancel_edit();
            self.asking = Some(cell);
            self.dont_ask = false;
        }
        let Some(asked) = self.asking else { return };
        let mut answer = None;
        let mut closed = false;
        house::dialog(ctx, "starsheets_protect_warning", 460.0, "Heads up!", Some(&mut closed), |ui| {
            house::text(ui, "You're trying to edit part of this sheet that shouldn't be changed accidentally. Edit anyway?");
            ui.checkbox(&mut self.dont_ask, "Don't show this again for 5 minutes");
            match house::actions(ui, &["Cancel", "OK"]) {
                Some(0) => answer = Some(false),
                Some(_) => answer = Some(true),
                None => {}
            }
        });
        if closed {
            answer = Some(false);
        }
        match answer {
            Some(true) => {
                // Allowed: the edit starts again, and the warning rests for a moment (or 5 min).
                if self.dont_ask {
                    self.quiet_until = now + 300.0;
                }
                self.asking = None;
                self.allowed = Some(asked);
                self.start_edit = true;
                ctx.request_repaint();
            }
            Some(false) => self.asking = None,
            None => {}
        }
    }
}

fn except_contains(except: &str, c: CellRef) -> bool {
    except.split(',').filter_map(|p| RangeRef::parse(&p.trim().replace('$', ""))).any(|r| r.contains(c))
}

/// Makes a sheet's real protection match its "Restrict editing" entries.
fn sync(app: &mut SheetApp, sheet: usize) {
    let list: Vec<Entry> = entries(app).into_iter().filter(|e| e.sheet == sheet && !e.meta.warn).collect();
    let Some(back) = app.session.active().map(|d| d.wb.active_sheet()) else { return };
    if back != sheet {
        let _ = app.run("sheet.activate", json!({ "sheet": sheet }));
    }
    let protected = app.session.active().and_then(|d| d.wb.sheets.get(sheet).map(|s| s.is_protected())).unwrap_or(false);
    if protected {
        let _ = app.run("review.unprotectSheet", json!({}));
    }
    let _ = app.run("review.allowEditRanges", json!({ "action": "delete", "title": FREE_TITLE }));
    if !list.is_empty() {
        let free: Vec<RangeRef> = match list.iter().find(|e| e.meta.sheet) {
            Some(whole) => whole.meta.except.split(',').filter_map(|p| RangeRef::parse(&p.trim().replace('$', ""))).collect(),
            None => complement(&list.iter().map(|e| e.range).collect::<Vec<_>>()),
        };
        if !free.is_empty() {
            let range = free.iter().map(RangeRef::a1).collect::<Vec<_>>().join(",");
            app.run_or_alert("review.allowEditRanges", json!({ "action": "add", "title": FREE_TITLE, "range": range }));
        }
        app.run_or_alert(
            "review.protectSheet",
            json!({ "formatCells": true, "formatColumns": true, "formatRows": true, "insertRows": true, "insertColumns": true, "deleteRows": true, "deleteColumns": true, "sort": true, "autofilter": true }),
        );
    }
    if back != sheet {
        let _ = app.run("sheet.activate", json!({ "sheet": back }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_come_from_the_section_and_older_files_still_read() {
        let mut s = gridcraft_engine::Session::new();
        s.execute("file.new", json!({})).unwrap();
        let old = Meta { d: "Old".into(), sheet: false, warn: true, except: String::new() };
        s.execute(
            "formulas.defineName",
            json!({ "name": format!("{PREFIX}1"), "refersTo": "=Sheet1!$A$1:$B$2", "comment": serde_json::to_string(&old).unwrap(), "force": true }),
        )
        .unwrap();
        s.execute(
            "formulas.defineName",
            json!({ "name": format!("{PREFIX}2"), "refersTo": "=Sheet1!$C$1:$C$2", "comment": "Totals", "force": true }),
        )
        .unwrap();
        let mut h = crate::hostdata::HostData::default();
        h.set_protect(
            &format!("{PREFIX}2"),
            serde_json::to_value(Meta { d: "Totals".into(), sheet: false, warn: false, except: "C2".into() }).unwrap(),
        );
        s.execute("workbook.setHostData", json!({ "data": serde_json::to_string(&h).unwrap() })).unwrap();
        let app = SheetApp::new(s, gridcraft_ui_egui::Services::default());
        let e = entries(&app);
        let (a, b) = (e.iter().find(|x| x.name.ends_with('1')).unwrap(), e.iter().find(|x| x.name.ends_with('2')).unwrap());
        assert!(a.meta.warn && a.meta.d == "Old", "an older file: settings from the comment");
        assert!(!b.meta.warn && b.meta.except == "C2", "from the section");
    }

    #[test]
    fn an_entry_is_kept_as_a_hidden_name_with_its_settings() {
        let mut s = gridcraft_engine::Session::new();
        s.execute("file.new", json!({})).unwrap();
        let meta = Meta { d: "Totals".into(), sheet: true, warn: false, except: "A1:B2".into() };
        s.execute(
            "formulas.defineName",
            json!({ "name": format!("{PREFIX}1"), "refersTo": "=Sheet1!$A$1:$XFD$1048576", "comment": serde_json::to_string(&meta).unwrap(), "force": true }),
        )
        .unwrap();
        let n = s.active().unwrap().wb.names.iter().find(|n| n.name.starts_with(PREFIX)).cloned().unwrap();
        let (_, body) = n.formula.trim_start_matches('=').rsplit_once('!').unwrap();
        assert!(RangeRef::parse(&body.replace('$', "")).is_some(), "{body}");
        let back: Meta = serde_json::from_str(&n.comment).unwrap();
        assert!(back.sheet && back.d == "Totals" && back.except == "A1:B2");
    }

    #[test]
    fn the_complement_covers_everything_else() {
        let taken = [RangeRef::parse("B2:C3").unwrap()];
        let free = complement(&taken);
        let hit = |a1: &str| {
            let c = CellRef::parse(a1).unwrap();
            free.iter().any(|r| r.contains(c))
        };
        assert!(!hit("B2") && !hit("C3"));
        assert!(hit("A1") && hit("A2") && hit("D2") && hit("B4") && hit("XFD1048576"));
    }

    #[test]
    fn a_protected_range_refuses_edits_and_the_rest_stays_editable() {
        let mut s = gridcraft_engine::Session::new();
        s.execute("file.new", json!({})).unwrap();
        let free: Vec<String> = complement(&[RangeRef::parse("B2:C3").unwrap()]).iter().map(RangeRef::a1).collect();
        s.execute("review.allowEditRanges", json!({ "action": "add", "title": FREE_TITLE, "range": free.join(",") })).unwrap();
        s.execute("review.protectSheet", json!({})).unwrap();
        assert!(s.execute("cell.set", json!({ "cell": "B2", "input": "no" })).is_err());
        assert!(s.execute("cell.set", json!({ "cell": "A1", "input": "yes" })).is_ok());
    }
}
