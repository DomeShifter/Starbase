//! Copy to ▸ New spreadsheet / Existing spreadsheet… (a sheet tab's right-click menu), as in
//! Google Sheets. The sheet travels as a one-sheet .xlsx made with starsheets_sync::copy_sheet_into
//! (its formats come along). A new spreadsheet is saved next to this one; an existing one is
//! picked from a list with search, and the server adds the sheet through that file's live room,
//! so anyone who has it open sees the new tab straight away (POST /api/files/ID/add-sheets).

use std::cell::RefCell;
use std::rc::Rc;

use egui::{RichText, Sense, vec2};
use gridcraft_ui_egui::SheetApp;
use serde_json::{Value as Json, json};

use crate::files::{Method, enc, fetch};
use crate::house;
use crate::theme;

/// Asks for Copy to: (sheet index, to an existing spreadsheet).
pub const REQUEST: &str = "starsheets_copy_to";

struct FileRow {
    id: String,
    name: String,
    folder: String,
}

enum Reply {
    Files(Vec<FileRow>),
    Done(String, Option<String>),
    Error(String),
}

#[derive(Default)]
pub struct CopyTo {
    inbox: Rc<RefCell<Vec<Reply>>>,
    /// Picking the spreadsheet to copy to: the sheet, the list, what's typed, the one chosen.
    picking: Option<(usize, String)>,
    files: Vec<FileRow>,
    loading: bool,
    search: String,
    chosen: Option<String>,
    busy: bool,
    /// Finished: what to say, and the spreadsheet to offer to open.
    done: Option<(String, Option<String>)>,
    error: Option<String>,
}

fn call(
    inbox: &Rc<RefCell<Vec<Reply>>>,
    ctx: &egui::Context,
    method: Method,
    url: String,
    body: Option<Vec<u8>>,
    then: impl FnOnce(Json) -> Reply + 'static,
) {
    let (inbox, ctx) = (inbox.clone(), ctx.clone());
    wasm_bindgen_futures::spawn_local(async move {
        let reply = match fetch(method, &url, body, None).await {
            Ok((s, _, bytes)) if s < 400 => then(serde_json::from_slice(&bytes).unwrap_or_default()),
            Ok((s, _, bytes)) => {
                let v: Json = serde_json::from_slice(&bytes).unwrap_or_default();
                Reply::Error(crate::home::said(&v, s))
            }
            Err(e) => Reply::Error(e),
        };
        inbox.borrow_mut().push(reply);
        ctx.request_repaint();
    });
}

/// Sheet `i` of the open spreadsheet as a one-sheet .xlsx file, formats included.
fn sheet_bytes(app: &SheetApp, i: usize) -> Result<(String, Vec<u8>), String> {
    let src = app.session.active().ok_or("There's no spreadsheet open.")?.wb.clone();
    let mut s = gridcraft_engine::Session::new();
    s.execute("file.new", json!({})).map_err(|e| e.to_string())?;
    let mut wb = (*s.active().ok_or("no workbook")?.wb).clone();
    wb.sheets.clear();
    let name = starsheets_sync::copy_sheet_into(&src, i, &mut wb).ok_or("That sheet isn't there any more.")?;
    let bytes = crate::files::xlsx(&wb, "copy.xlsx")?;
    Ok((name, bytes))
}

fn open_tab(id: &str) {
    if let Some(w) = web_sys::window() {
        let path = w.location().pathname().unwrap_or_else(|_| "/".into());
        let _ = w.open_with_url_and_target(&format!("{path}?file={id}"), "_blank");
    }
}

impl CopyTo {
    pub fn logic(&mut self, app: &SheetApp, ctx: &egui::Context, files: &crate::files::Files) {
        for r in std::mem::take(&mut *self.inbox.borrow_mut()) {
            self.busy = false;
            match r {
                Reply::Files(f) => {
                    self.files = f;
                    self.loading = false;
                }
                Reply::Done(text, open) => {
                    self.picking = None;
                    self.done = Some((text, open));
                }
                Reply::Error(e) => {
                    self.loading = false;
                    self.error = Some(e);
                    if self.picking.is_none() {
                        self.done = Some((format!("The sheet wasn't copied: {}", self.error.take().unwrap_or_default()), None));
                    }
                }
            }
        }
        let Some((sheet, existing)) = ctx.data_mut(|m| m.remove_temp::<(usize, bool)>(egui::Id::unique(REQUEST))) else { return };
        let this = files.linked().map(|(_, id)| id).unwrap_or_default();
        if existing {
            let name = app.session.active().and_then(|d| d.wb.sheets.get(sheet)).map(|s| s.name.clone()).unwrap_or_default();
            self.picking = Some((sheet, name));
            self.search.clear();
            self.chosen = None;
            self.error = None;
            self.loading = true;
            call(&self.inbox, ctx, Method::Get, "api/files".into(), None, move |v| {
                Reply::Files(
                    v["files"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|f| f["id"].as_str() != Some(this.as_str()) && crate::home::may_edit(f["role"].as_str().unwrap_or("")))
                        // Only types that hold several sheets.
                        .filter(|f| f["name"].as_str().is_some_and(|n| [".xlsx", ".xlsm", ".ods"].iter().any(|e| n.to_lowercase().ends_with(e))))
                        .map(|f| FileRow {
                            id: f["id"].as_str().unwrap_or("").into(),
                            name: f["name"].as_str().unwrap_or("").into(),
                            folder: f["folder"].as_str().unwrap_or("").into(),
                        })
                        .collect(),
                )
            });
            return;
        }
        // A new spreadsheet, next to this one, named after the sheet.
        match sheet_bytes(app, sheet) {
            Ok((name, bytes)) => {
                let folder = files.folder().unwrap_or_default();
                let title = format!("Copy of {name}");
                call(&self.inbox, ctx, Method::Post, format!("api/files?name={}&folder={}", enc(&title), enc(&folder)), Some(bytes), move |v| {
                    Reply::Done(
                        format!("\u{201c}{name}\u{201d} was copied to a new spreadsheet, \u{201c}{title}\u{201d}."),
                        v["id"].as_str().map(str::to_string),
                    )
                });
            }
            Err(e) => self.done = Some((format!("The sheet wasn't copied: {e}"), None)),
        }
    }

    pub fn ui(&mut self, app: &SheetApp, ctx: &egui::Context) {
        if let Some((sheet, name)) = self.picking.clone() {
            let mut close = false;
            let mut cancel = false;
            let mut go = false;
            house::dialog(ctx, "starsheets_copy_to", 520.0, &format!("Copy \u{201c}{name}\u{201d} to…"), Some(&mut close), |ui| {
                house::note(ui, "Pick the spreadsheet to add this sheet to. It goes in as a new tab at the end.");
                ui.add_space(house::S);
                let r = ui.add(
                    house::field(&mut self.search, house::BODY)
                        .hint_text("Search spreadsheets")
                        .desired_width(f32::INFINITY)
                        .min_size(vec2(0.0, house::FIELD_HEIGHT)),
                );
                if !r.has_focus() && self.search.is_empty() && !self.busy {
                    r.request_focus();
                }
                ui.add_space(house::S);
                let q = self.search.trim().to_lowercase();
                egui::Frame::NONE.stroke(egui::Stroke::new(1.0, theme::DIVIDER)).corner_radius(8).inner_margin(egui::Margin::same(4)).show(
                    ui,
                    |ui| {
                        egui::ScrollArea::vertical().max_height(300.0).min_scrolled_height(300.0).auto_shrink([false, false]).show(ui, |ui| {
                            if self.loading {
                                ui.label(RichText::new("Loading…").color(theme::TEXT_DIM));
                            }
                            // Matches the name or the folder it's in.
                            let shown: Vec<&FileRow> = self
                                .files
                                .iter()
                                .filter(|f| q.is_empty() || f.name.to_lowercase().contains(&q) || f.folder.to_lowercase().contains(&q))
                                .collect();
                            if !self.loading && shown.is_empty() {
                                ui.label(RichText::new("No spreadsheets match.").color(theme::TEXT_DIM));
                            }
                            for f in shown {
                                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                                let p = ui.painter().clone();
                                let picked = self.chosen.as_deref() == Some(f.id.as_str());
                                if picked {
                                    p.rect_filled(rect, 6, theme::SHARE);
                                } else if resp.hovered() {
                                    p.rect_filled(rect, 6, theme::MENU_HOVER);
                                }
                                if let Some(img) = crate::icons::icon("starsheets") {
                                    img.paint_at(ui, egui::Rect::from_center_size(rect.left_center() + vec2(18.0, 0.0), vec2(18.0, 18.0)));
                                }
                                p.text(
                                    rect.left_center() + vec2(38.0, -7.0),
                                    egui::Align2::LEFT_CENTER,
                                    &f.name,
                                    egui::FontId::proportional(14.0),
                                    theme::TEXT,
                                );
                                let place = f.folder.split('/').skip(1).collect::<Vec<_>>().join(" › ");
                                p.text(
                                    rect.left_center() + vec2(38.0, 9.0),
                                    egui::Align2::LEFT_CENTER,
                                    if place.is_empty() { "In the top folder".to_string() } else { format!("In {place}") },
                                    egui::FontId::proportional(11.0),
                                    theme::TEXT_DIM,
                                );
                                if resp.clicked() {
                                    self.chosen = Some(f.id.clone());
                                }
                                if resp.double_clicked() {
                                    go = true;
                                }
                            }
                        });
                    },
                );
                if let Some(e) = &self.error {
                    ui.label(RichText::new(e).size(house::NOTE).color(house::ERROR));
                }
                match house::actions(ui, &["Cancel", "Copy"]) {
                    Some(0) => cancel = true,
                    Some(_) => go = true,
                    None => {}
                }
            });
            if close || cancel {
                self.picking = None;
            } else if go && !self.busy {
                match (self.chosen.clone(), sheet_bytes(app, sheet)) {
                    (None, _) => self.error = Some("Pick a spreadsheet first.".into()),
                    (Some(_), Err(e)) => self.error = Some(e),
                    (Some(id), Ok((_, bytes))) => {
                        self.busy = true;
                        let target = self.files.iter().find(|f| f.id == id).map_or_else(String::new, |f| f.name.clone());
                        call(&self.inbox, ctx, Method::Post, format!("api/files/{id}/add-sheets?name=copy.xlsx"), Some(bytes), move |v| {
                            let as_name = v["sheets"].as_array().and_then(|a| a.first()).and_then(|s| s.as_str()).unwrap_or("").to_string();
                            Reply::Done(
                                format!("\u{201c}{name}\u{201d} was copied to \u{201c}{target}\u{201d} as the tab \u{201c}{as_name}\u{201d}."),
                                Some(id),
                            )
                        });
                    }
                }
            }
        }
        if let Some((text, open)) = self.done.clone() {
            let mut close = false;
            let mut pick = None;
            house::dialog(ctx, "starsheets_copy_done", 420.0, "Copy to", Some(&mut close), |ui| {
                house::text(ui, &text);
                pick = house::actions(ui, if open.is_some() { &["OK", "Open spreadsheet"] } else { &["OK"] });
            });
            if pick == Some(1)
                && let Some(id) = &open
            {
                open_tab(id);
            }
            if close || pick.is_some() {
                self.done = None;
            }
        }
    }
}
