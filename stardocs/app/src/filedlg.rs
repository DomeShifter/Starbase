//! File › Move, Move to bin and Details for a document on the Starbase server, as in Sheets'
//! File menu and the Drive: a folder picker (the same `api/files/<id>/move?folder=` call the
//! Drive's "Move to…" makes), a confirmation before the document goes to the bin, and a small
//! card of facts (owner, size, last changed, folder) read from the server's file list.

use std::cell::RefCell;
use std::rc::Rc;

use egui::{Align, Color32, Layout, RichText, vec2};
use serde_json::Value as Json;
use wordcraft_ui_egui::WordApp;

use crate::server::{Link, api, fetch};
use crate::shell::{BLUE, SHARE, TEXT, TEXT_DIM};

const ERROR: Color32 = Color32::from_rgb(0xB3, 0x26, 0x1E);

#[derive(Clone, PartialEq)]
enum Kind {
    Move,
    Bin,
    Details,
}

struct Facts {
    owner: String,
    size: u64,
    modified: f64,
    folder: String,
    /// My own folder: where "My files" is.
    space: String,
    /// Folders I may add to: (path, is it where the document already is).
    folders: Vec<String>,
}

enum Reply {
    Loaded(Facts),
    Moved(String),
    Binned,
    Error(String),
}

#[derive(Default)]
pub struct FileDlg {
    inbox: Rc<RefCell<Vec<Reply>>>,
    kind: Option<Kind>,
    id: String,
    name: String,
    facts: Option<Facts>,
    target: Option<String>,
    error: Option<String>,
    busy: bool,
}

fn call(inbox: &Rc<RefCell<Vec<Reply>>>, ctx: &egui::Context, method: &'static str, url: String, then: impl FnOnce(Json) -> Reply + 'static) {
    let (inbox, ctx) = (inbox.clone(), ctx.clone());
    wasm_bindgen_futures::spawn_local(async move {
        let reply = match fetch(method, &url, None, None).await {
            Ok((s, _, bytes)) => {
                let v: Json = serde_json::from_slice(&bytes).unwrap_or_default();
                if s < 400 { then(v) } else { Reply::Error(v["error"].as_str().map_or(format!("The server said {s}."), str::to_string)) }
            }
            Err(e) => Reply::Error(e),
        };
        inbox.borrow_mut().push(reply);
        ctx.request_repaint();
    });
}

fn enc(s: &str) -> String {
    js_sys::encode_uri_component(s).as_string().unwrap_or_default()
}

impl FileDlg {
    /// Opens `kind` ("move", "bin" or "details") for the document.
    pub fn ask(&mut self, kind: &str, app: &mut WordApp, link: Option<&Link>, ctx: &egui::Context) {
        let Some(link) = link else {
            app.status("This needs a Starbase server: open the document from the home screen");
            return;
        };
        let Some(id) = link.id() else {
            app.status("The document isn't saved yet");
            return;
        };
        let kind = match kind {
            "move" => Kind::Move,
            "bin" => Kind::Bin,
            _ => Kind::Details,
        };
        let name = Some(link.name()).filter(|n| !n.is_empty()).unwrap_or_else(|| "Untitled document".to_string());
        *self = FileDlg { inbox: self.inbox.clone(), kind: Some(kind.clone()), id: id.clone(), name, ..FileDlg::default() };
        if kind != Kind::Bin {
            let me = id.clone();
            call(&self.inbox, ctx, "GET", api("api/files"), move |v| {
                let file = v["files"].as_array().into_iter().flatten().find(|f| f["id"].as_str() == Some(me.as_str())).cloned().unwrap_or_default();
                let owner = file["owner"].as_str().unwrap_or("").to_string();
                let owner = v["people"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|p| p["username"].as_str() == Some(owner.as_str()))
                    .and_then(|p| p["name"].as_str())
                    .map_or(owner, str::to_string);
                let can_add = |f: &str| matches!(v["folder_info"][f]["role"].as_str(), Some("owner" | "edit"));
                let mut folders: Vec<String> =
                    v["folders"].as_array().into_iter().flatten().filter_map(|f| f.as_str()).filter(|f| can_add(f)).map(str::to_string).collect();
                folders.sort();
                Reply::Loaded(Facts {
                    owner,
                    size: file["size"].as_u64().unwrap_or(0),
                    modified: file["modified"].as_f64().unwrap_or(0.0),
                    folder: file["folder"].as_str().unwrap_or("").to_string(),
                    space: v["space"].as_str().unwrap_or("").to_string(),
                    folders,
                })
            });
        }
    }

    /// Every frame: the server's replies.
    pub fn logic(&mut self, link: Option<&Link>, _ctx: &egui::Context) {
        let replies: Vec<Reply> = std::mem::take(&mut *self.inbox.borrow_mut());
        for r in replies {
            self.busy = false;
            match r {
                Reply::Loaded(f) => {
                    self.target = (self.kind == Some(Kind::Move)).then(|| f.folder.clone());
                    self.facts = Some(f);
                }
                Reply::Moved(to) => {
                    self.kind = None;
                    if let Some(l) = link {
                        l.news(format!("Moved to {}", if to.is_empty() { "your files" } else { &to }));
                    }
                }
                Reply::Binned => {
                    self.kind = None;
                    if let Some(l) = link {
                        l.home();
                    }
                }
                Reply::Error(e) => self.error = Some(e),
            }
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.kind.clone() else { return };
        let mut close = false;
        let mut go = false;
        let id = egui::Id::unique("stardocs_filedlg");
        let top = (ctx.content_rect().height() * 0.12).clamp(24.0, 120.0);
        let modal = egui::Modal::new(id)
            .area(egui::Modal::default_area(id).anchor(egui::Align2::CENTER_TOP, vec2(0.0, top)))
            .frame(egui::Frame::popup(&ctx.global_style()).fill(Color32::WHITE).corner_radius(8.0).inner_margin(24.0));
        let shown = modal.show(ctx, |ui| {
            ui.set_width(440.0);
            let title = match kind {
                Kind::Move => format!("Move \"{}\"", self.name),
                Kind::Bin => "Move to bin".to_string(),
                Kind::Details => "Details".to_string(),
            };
            ui.label(RichText::new(title).size(22.0).color(TEXT));
            ui.add_space(10.0);
            match (&kind, &self.facts) {
                (Kind::Bin, _) => {
                    ui.label(
                        RichText::new(format!("Move \"{}\" to the bin? You can restore it from the Bin on your home screen.", self.name))
                            .size(14.0)
                            .color(TEXT),
                    );
                    ui.label("")
                }
                (_, None) if self.error.is_none() => ui.label(RichText::new("Loading…").size(12.0).color(TEXT_DIM)),
                (_, None) => ui.label(""),
                (Kind::Details, Some(f)) => {
                    let place = if f.folder.is_empty() || f.folder == f.space { "My files".to_string() } else { f.folder.replace('/', " › ") };
                    let rows = [
                        ("Name", self.name.clone()),
                        ("Owner", f.owner.clone()),
                        ("Size", size_text(f.size)),
                        ("Modified", when(f.modified)),
                        ("Location", place),
                    ];
                    egui::Grid::new("stardocs_details_grid").num_columns(2).spacing(vec2(16.0, 8.0)).show(ui, |ui| {
                        for (k, v) in rows {
                            ui.label(RichText::new(k).size(14.0).color(TEXT_DIM));
                            ui.label(RichText::new(v).size(14.0).color(TEXT));
                            ui.end_row();
                        }
                    });
                    ui.label("")
                }
                (Kind::Move, Some(f)) => {
                    let current = f.folder.clone();
                    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                        let mut rows: Vec<(String, String)> = vec![(f.space.clone(), "My files".to_string())];
                        for p in f.folders.iter().filter(|p| **p != f.space) {
                            let depth = p.matches('/').count();
                            let last = p.rsplit('/').next().unwrap_or(p);
                            rows.push((p.clone(), format!("{}{}", "      ".repeat(depth), last)));
                        }
                        for (path, label) in rows {
                            let picked = self.target.as_deref() == Some(path.as_str());
                            let text = if path == current { format!("{label}  (here)") } else { label };
                            if ui.selectable_label(picked, RichText::new(text).size(14.0).color(TEXT)).clicked() {
                                self.target = Some(path);
                            }
                        }
                    });
                    ui.label("")
                }
            };
            if let Some(e) = &self.error {
                ui.add_space(6.0);
                ui.label(RichText::new(e).size(12.0).color(ERROR));
            }
            ui.add_space(16.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| match kind {
                Kind::Details => {
                    if ui.add(button("Close", SHARE, TEXT)).clicked() {
                        close = true;
                    }
                }
                _ => {
                    let label = if kind == Kind::Bin { "Move to bin" } else { "Move here" };
                    let ready = !self.busy
                        && (kind == Kind::Bin || self.target.as_ref().is_some_and(|t| self.facts.as_ref().is_some_and(|f| *t != f.folder)));
                    if ui.add_enabled(ready, button(label, BLUE, Color32::WHITE)).clicked() {
                        go = true;
                    }
                    if ui.add(button("Cancel", Color32::WHITE, BLUE)).clicked() {
                        close = true;
                    }
                }
            });
        });
        if close || shown.should_close() {
            self.kind = None;
        } else if go {
            self.busy = true;
            self.error = None;
            if kind == Kind::Bin {
                call(&self.inbox, ctx, "DELETE", api(&format!("api/files/{}", self.id)), |_| Reply::Binned);
            } else if let Some(to) = self.target.clone() {
                call(&self.inbox, ctx, "POST", api(&format!("api/files/{}/move?folder={}", self.id, enc(&to))), move |_| Reply::Moved(to));
            }
        }
    }
}

fn button(text: &str, fill: Color32, colour: Color32) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).size(14.0).color(colour)).fill(fill).corner_radius(18.0).min_size(vec2(84.0, 36.0))
}

/// "12 KB", "3.4 MB".
fn size_text(n: u64) -> String {
    match n {
        0..=999 => format!("{n} bytes"),
        1000..=999_999 => format!("{:.0} KB", n as f64 / 1e3),
        _ => format!("{:.1} MB", n as f64 / 1e6),
    }
}

/// A server time (seconds since 1970) in the reader's own way of writing dates.
fn when(secs: f64) -> String {
    if secs <= 0.0 {
        return "Unknown".to_string();
    }
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(secs * 1000.0));
    d.to_locale_string("en-GB", &wasm_bindgen::JsValue::UNDEFINED).as_string().unwrap_or_default()
}
