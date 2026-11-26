//! People beside the photo grid: faces grouped by the server (crates/server/src/photos/faces.rs)
//! as round crops; name a person, merge two, take a wrong face out, open a person's pictures.
//! Named people first, unnamed groups below.

use egui::{Color32, Pos2, Rect, RichText, Sense, TextureId, Ui, pos2, vec2};
use serde_json::{Value as Json, json};

use super::albums::{Card, Tab};
use super::{Got, PhotoMode, SMALL, link};
use crate::house;
use crate::store::{Backend, Method};
use crate::theme;

/// One face in an opened person.
#[derive(Clone, Debug)]
pub(in crate::home) struct FaceCard {
    pub face: i64,
    pub id: String,
    pub v: String,
    pub bx: [f32; 4],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Ask {
    #[default]
    None,
    Name,
    Merge,
}

#[derive(Default)]
pub(in crate::home) struct People {
    pub asked: bool,
    pub named: Vec<Card>,
    pub groups: Vec<Card>,
    /// The list's state: on, note, missing models, progress.
    pub state: Json,
    pub faces: Vec<FaceCard>,
    text: String,
    ask: Ask,
    /// When the list was last asked for again while models download (seconds, egui time).
    poll: f64,
    /// "Scan now" (scan.rs): in the toolbar and on the People page.
    pub scan: super::scan::ScanNow,
}

impl People {
    pub fn take(&mut self, v: &Json) {
        self.named = super::albums::cards(v, "people");
        self.groups = super::albums::cards(v, "groups");
        self.state = v.clone();
    }

    pub fn take_faces(&mut self, v: &Json) {
        self.faces = v["faces"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| {
                let b = f["box"].as_array()?;
                let n = |i: usize| b.get(i).and_then(Json::as_f64).unwrap_or(0.0) as f32;
                Some(FaceCard {
                    face: f["face"].as_i64()?,
                    id: f["id"].as_str()?.to_string(),
                    v: f["v"].as_str()?.to_string(),
                    bx: [n(0), n(1), n(2), n(3)],
                })
            })
            .collect();
    }

    #[cfg(target_arch = "wasm32")]
    pub fn test_state(&self) -> Json {
        let c = |v: &Vec<Card>| v.iter().map(|c| json!({"id": c.id, "name": c.name, "count": c.count})).collect::<Vec<_>>();
        json!({"named": c(&self.named), "groups": c(&self.groups), "on": self.state["on"], "faces": self.faces.len(), "can_download": self.state["can_download"], "download": self.state["download"], "missing": self.state["missing"].as_array().map_or(0, Vec::len), "progress": self.state["progress"], "scan": self.scan.test_state()})
    }
}

/// The part of a thumbnail `[tw, th]` around face box `bx`, square, with some room around it.
pub(in crate::home) fn face_uv(bx: [f32; 4], tw: f32, th: f32) -> Rect {
    let (cx, cy) = (bx[0] + bx[2] / 2.0, bx[1] + bx[3] / 2.0);
    let side = (bx[2] * tw).max(bx[3] * th) * 1.5;
    let (hu, hv) = (side / 2.0 / tw.max(1.0), side / 2.0 / th.max(1.0));
    Rect::from_min_max(pos2(cx - hu, cy - hv), pos2(cx + hu, cy + hv))
}

/// A round crop of texture `tex` (part `uv`) in a circle.
fn round(p: &egui::Painter, tex: TextureId, c: Pos2, r: f32, uv: Rect) {
    let mut m = egui::Mesh::with_texture(tex);
    let cu = uv.center();
    m.vertices.push(egui::epaint::Vertex { pos: c, uv: cu, color: Color32::WHITE });
    const N: u32 = 48;
    for i in 0..=N {
        let (s, co) = (i as f32 / N as f32 * std::f32::consts::TAU).sin_cos();
        m.vertices.push(egui::epaint::Vertex {
            pos: c + vec2(co, s) * r,
            uv: pos2(cu.x + co * uv.width() / 2.0, cu.y + s * uv.height() / 2.0),
            color: Color32::WHITE,
        });
    }
    for i in 1..=N {
        m.add_triangle(0, i, i + 1);
    }
    p.add(m);
}

impl PhotoMode {
    pub(super) fn people_boot(&mut self, ctx: &egui::Context, store: &Backend) {
        if !self.alb.ppl.asked {
            self.alb.ppl.asked = true;
            super::albums::get(ctx, store, &self.inbox, "api/photos/people", |v| Got::Cards(Tab::People, v));
        }
    }

    /// After a change: the list and the opened one again (`id`: what the opened one is now).
    pub(super) fn people_changed(&mut self, ctx: &egui::Context, store: &Backend, id: Option<String>) {
        self.alb.ppl.asked = false;
        self.people_boot(ctx, store);
        if let (Some(id), Some(c)) = (id, self.alb.open.as_mut()) {
            c.id = id.clone();
            super::albums::get(ctx, store, &self.inbox, &format!("api/photos/people/{id}"), move |v| Got::Opened(Tab::People, id, v));
        }
    }

    fn people_post(&mut self, ctx: &egui::Context, store: &Backend, url: &str, body: Json) {
        let (inbox, ctx2) = (self.inbox.clone(), ctx.clone());
        store.call(
            ctx,
            Method::Post,
            url,
            Some(body.to_string().into_bytes()),
            Box::new(move |a| {
                let g = match a {
                    Ok((200, b)) => {
                        let v: Json = serde_json::from_slice(&b).unwrap_or_default();
                        Got::PersonIs(v["id"].as_str().map(str::to_string))
                    }
                    Ok((s, b)) => {
                        let v: Json = serde_json::from_slice(&b).unwrap_or_default();
                        Got::Error(v["error"].as_str().map_or(format!("The server said {s}."), str::to_string))
                    }
                    Err(e) => Got::Error(e),
                };
                PhotoMode::push(&inbox, &ctx2, g);
            }),
        );
    }

    /// A round face from picture `id` (content tag `v`), box `bx`, centred at `c`.
    fn face_ui(&mut self, ctx: &egui::Context, store: &Backend, p: &egui::Painter, (id, v, bx): (&str, &str, [f32; 4]), c: Pos2, r: f32) {
        p.circle_filled(c, r, Color32::from_rgb(0xE8, 0xEA, 0xED));
        self.want_id(ctx, store, id, Some(v), SMALL);
        let frame = self.frame;
        if let Some(t) = self.tex.get(&(id.to_string(), SMALL), frame) {
            let [w, h] = t.size();
            round(p, t.id(), c, r, face_uv(bx, w as f32, h as f32));
        }
    }

    fn person_card(&mut self, ui: &mut Ui, ctx: &egui::Context, store: &Backend, c: &Card, n: usize) -> bool {
        let side = 128.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(side, side + 40.0), Sense::click());
        house::target(ctx, &format!("person:{n}"), rect);
        let p = ui.painter_at(rect);
        let centre = rect.min + vec2(side / 2.0, side / 2.0);
        if let (Some((id, v)), Some(bx)) = (&c.cover, c.bx) {
            self.face_ui(ctx, store, &p, (id, v, bx), centre, side / 2.0 - 4.0);
        }
        if resp.hovered() {
            p.circle_stroke(centre, side / 2.0 - 3.0, egui::Stroke::new(3.0, theme::SHARE));
        }
        let name = if c.name.is_empty() { "Add a name".to_string() } else { c.name.clone() };
        let col = if c.name.is_empty() { theme::text_dim() } else { theme::text() };
        p.text(rect.min + vec2(side / 2.0, side + 4.0), egui::Align2::CENTER_TOP, name, egui::FontId::proportional(house::BODY), col);
        let n = format!("{} {}", c.count, if c.count == 1 { "picture" } else { "pictures" });
        p.text(rect.min + vec2(side / 2.0, side + 22.0), egui::Align2::CENTER_TOP, n, egui::FontId::proportional(house::NOTE), theme::text_dim());
        resp.clicked()
    }

    /// Ask the list again every `secs` seconds while faces are being looked for (or models fetched).
    fn people_poll(&mut self, ui: &Ui, secs: f64) {
        let now = ui.input(|i| i.time);
        if now - self.alb.ppl.poll > secs {
            self.alb.ppl.poll = now;
            self.alb.ppl.asked = false;
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(secs));
    }

    /// "Finding faces: 4 of 10" as a bar; true while the scan is running.
    fn scan_bar(&mut self, ui: &mut Ui, st: &Json, wide: bool) -> bool {
        let pr = &st["progress"];
        let (done, all) = (pr["done"].as_u64().unwrap_or(0), pr["all"].as_u64().unwrap_or(0));
        if done >= all {
            return false;
        }
        let bar = egui::ProgressBar::new(done as f32 / all.max(1) as f32)
            .desired_width(if wide { 320.0 } else { 220.0 })
            .text(RichText::new(format!("Finding faces: {done} of {all}")).size(house::NOTE));
        let r = ui.add(bar);
        house::target(ui.ctx(), "people:progress", r.rect);
        if wide {
            ui.label(RichText::new("You can keep using Photos. People appear here as they are found.").size(house::NOTE).color(theme::text_dim()));
        }
        true
    }

    /// Off, models missing, progress: one clear path to turn it on, then a progress bar.
    fn people_status(&mut self, ui: &mut Ui, ctx: &egui::Context, store: &Backend) {
        let st = self.alb.ppl.state.clone();
        let dim = |ui: &mut Ui, t: String| ui.label(RichText::new(t).size(house::NOTE).color(theme::text_dim()));
        if st.is_null() {
            return;
        }
        if st["on"].as_bool() != Some(true) {
            dim(ui, "Face recognition groups the same face across your pictures. It runs on this server only; nothing is sent anywhere.".into());
            let downloading = st["download"]["busy"].as_bool() == Some(true);
            if st["can_toggle"].as_bool() == Some(true) {
                if downloading || st["download"]["error"].is_string() {
                    self.models_ui(ui, ctx, store, &st);
                }
                let need = st["can_download"].as_bool() == Some(true) && st["missing"].as_array().is_some_and(|m| !m.is_empty());
                let label = if need { "Download face models and turn on" } else { "Turn on face recognition" };
                if !downloading && self.pill(ui, label, false) {
                    self.people_post(ctx, store, "api/photos/faces", json!({"on": true}));
                    if need {
                        self.people_post(ctx, store, "api/photos/faces/models", json!({}));
                    }
                }
            } else {
                dim(ui, "It is off. Ask the owner of this server to turn it on (in Photos, People).".into());
            }
            return;
        }
        self.models_ui(ui, ctx, store, &st);
        if let Some(m) = st["missing"].as_array().filter(|m| !m.is_empty() && st["can_download"].as_bool() != Some(true)) {
            let files: Vec<String> = m
                .iter()
                .map(|f| format!("{} ({}, {})", f["file"].as_str().unwrap_or(""), f["what"].as_str().unwrap_or(""), f["url"].as_str().unwrap_or("")))
                .collect();
            ui.label(
                RichText::new(format!(
                    "Model files are missing. Put them in {}: {}.",
                    st["models"].as_str().unwrap_or("the models folder"),
                    files.join("; ")
                ))
                .size(house::NOTE)
                .color(house::ERROR),
            );
        }
        let pr = &st["progress"];
        let waiting = st["missing"].as_array().is_some_and(|m| !m.is_empty());
        ui.horizontal(|ui| self.alb.ppl.scan.ui(ui, store));
        let scanning = self.scan_bar(ui, &st, true);
        if scanning || waiting {
            self.people_poll(ui, 2.0);
        }
        ui.horizontal(|ui| {
            if !scanning {
                dim(ui, format!("Done: {} faces found. {}", pr["faces"].as_u64().unwrap_or(0), st["note"].as_str().unwrap_or("")));
            } else if let Some(n) = st["note"].as_str().filter(|n| !n.is_empty()) {
                dim(ui, n.to_string());
            }
            if st["can_toggle"].as_bool() == Some(true) && self.pill(ui, "Turn off", false) {
                self.people_post(ctx, store, "api/photos/faces", json!({"on": false}));
            }
        });
        if st["can_toggle"].as_bool() == Some(true) {
            dim(ui, "Turning off stops the scan. Names and groups are kept.".into());
        }
    }

    /// Open a person's page (from the list or the strip).
    fn people_open(&mut self, ctx: &egui::Context, store: &Backend, c: Card) {
        self.pics.clear();
        self.viewer = None;
        self.alb.ppl.faces.clear();
        self.alb.ppl.ask = Ask::None;
        self.loading = true;
        let key = c.id.clone();
        self.alb.open = Some(c);
        super::albums::get(ctx, store, &self.inbox, &format!("api/photos/people/{key}"), move |v| Got::Opened(Tab::People, key, v));
    }

    /// A row of round faces at the top of Photos; a tap opens the person, "See all" opens People.
    pub(in crate::home) fn people_strip(&mut self, ui: &mut Ui, store: &Backend) {
        let ctx = ui.ctx().clone();
        self.people_boot(&ctx, store);
        let st = self.alb.ppl.state.clone();
        if st["on"].as_bool() != Some(true) {
            return;
        }
        if self.scan_bar(ui, &st, false) {
            self.people_poll(ui, 3.0);
        }
        let mut all = self.alb.ppl.named.clone();
        all.extend(self.alb.ppl.groups.iter().filter(|c| c.count > 1).cloned());
        if all.is_empty() {
            return;
        }
        let (side, gap) = (64.0, house::M);
        let fit = (((ui.available_width() - 90.0) / (side + gap)) as usize).clamp(1, 12);
        let mut open = None;
        let mut see_all = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (n, c) in all.iter().take(fit).enumerate() {
                let (rect, resp) = ui.allocate_exact_size(vec2(side, side + 20.0), Sense::click());
                house::target(&ctx, &format!("people-strip:{n}"), rect);
                let p = ui.painter_at(rect);
                let centre = rect.min + vec2(side / 2.0, side / 2.0);
                if let (Some((id, v)), Some(bx)) = (&c.cover, c.bx) {
                    self.face_ui(&ctx, store, &p, (id, v, bx), centre, side / 2.0 - 2.0);
                }
                if resp.hovered() {
                    p.circle_stroke(centre, side / 2.0 - 1.0, egui::Stroke::new(2.0, theme::SHARE));
                }
                let name = if c.name.is_empty() { "Unnamed".to_string() } else { c.name.clone() };
                p.text(
                    rect.min + vec2(side / 2.0, side + 4.0),
                    egui::Align2::CENTER_TOP,
                    name,
                    egui::FontId::proportional(house::NOTE),
                    theme::text_dim(),
                );
                if resp.clicked() {
                    open = Some(c.clone());
                }
            }
            if self.pill(ui, "See all", false) {
                see_all = true;
            }
        });
        if let Some(c) = open {
            self.go(super::albums::Tab::People);
            self.people_open(&ctx, store, c);
        } else if see_all {
            self.go(super::albums::Tab::People);
        }
        ui.add_space(house::S);
    }

    /// The owner's "Download face models" button, its progress and errors.
    fn models_ui(&mut self, ui: &mut Ui, ctx: &egui::Context, store: &Backend, st: &Json) {
        if st["can_download"].as_bool() != Some(true) {
            return;
        }
        let d = &st["download"];
        let mb = |b: u64| b as f64 / 1_048_576.0;
        if d["busy"].as_bool() == Some(true) {
            let (got, total) = (d["got"].as_u64().unwrap_or(0), d["total"].as_u64().unwrap_or(0).max(1));
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::ProgressBar::new(got as f32 / total as f32).desired_width(160.0));
                ui.label(
                    RichText::new(format!("Downloading {}: {:.0} of {:.0} MB", d["file"].as_str().unwrap_or("the models"), mb(got), mb(total)))
                        .size(house::NOTE)
                        .color(theme::text_dim()),
                );
            });
            // Ask again every second while it runs.
            let now = ui.input(|i| i.time);
            if now - self.alb.ppl.poll > 1.0 {
                self.alb.ppl.poll = now;
                self.alb.ppl.asked = false;
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
            return;
        }
        if let Some(e) = d["error"].as_str() {
            ui.label(RichText::new(format!("The download failed: {e}")).size(house::NOTE).color(house::ERROR));
        }
        let files: Vec<String> = st["missing"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|f| format!("{} ({})", f["what"].as_str().unwrap_or(""), f["licence"].as_str().unwrap_or("")))
            .collect();
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("Face recognition needs model files: {}.", files.join(", "))).size(house::NOTE).color(theme::text_dim()));
            if self.pill(ui, "Download face models", false) {
                self.people_post(ctx, store, "api/photos/faces/models", json!({}));
            }
        });
    }

    /// The people and groups, or the opened one.
    pub(super) fn people_ui(&mut self, ui: &mut Ui, store: &Backend) {
        let ctx = ui.ctx().clone();
        self.people_boot(&ctx, store);
        if let Some(e) = self.error.clone() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(e).size(house::NOTE).color(house::ERROR));
                if ui.add(house::link("OK")).clicked() {
                    self.error = None;
                }
            });
        }
        if let Some(c) = self.alb.open.clone() {
            self.person_ui(ui, &ctx, store, &c);
            return;
        }
        ui.label(RichText::new("People").size(house::TITLE).color(theme::text()));
        self.people_status(ui, &ctx, store);
        ui.add_space(house::S);
        let (named, groups) = (self.alb.ppl.named.clone(), self.alb.ppl.groups.clone());
        if named.is_empty() && groups.is_empty() {
            ui.add_space(house::XL);
            let on = self.alb.ppl.state["on"].as_bool() == Some(true);
            let scanning = self.alb.ppl.state["progress"]["done"].as_u64() < self.alb.ppl.state["progress"]["all"].as_u64();
            let t = if !on {
                ""
            } else if scanning {
                "Still looking. People appear here as they are found."
            } else {
                "No faces found in these pictures."
            };
            ui.label(RichText::new(t).size(house::BODY).color(theme::text_dim()));
            return;
        }
        let (mut open, mut n) = (None, 0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (title, list) in [("", &named), ("Not named yet", &groups)] {
                if list.is_empty() {
                    continue;
                }
                if !title.is_empty() {
                    ui.add_space(house::L);
                    ui.label(RichText::new(title).size(house::BODY).color(theme::text_dim()));
                }
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(house::L, house::L);
                    for c in list {
                        n += 1;
                        if self.person_card(ui, &ctx, store, c, n - 1) {
                            open = Some(c.clone());
                        }
                    }
                });
            }
        });
        if let Some(c) = open {
            self.people_open(&ctx, store, c);
        }
    }

    fn person_ui(&mut self, ui: &mut Ui, ctx: &egui::Context, store: &Backend, c: &Card) {
        let url = format!("api/photos/people/{}", c.id);
        ui.horizontal(|ui| {
            let back = ui.add(link("‹ People"));
            house::target(ctx, "people:back", back.rect);
            if back.clicked() {
                self.alb.open = None;
                self.alb.ppl.ask = Ask::None;
                self.pics.clear();
                self.viewer = None;
            }
            let title = if c.name.is_empty() { "Not named yet" } else { c.name.as_str() };
            ui.label(RichText::new(title).size(house::TITLE).color(theme::text()));
            if self.pill(ui, if c.name.is_empty() { "Add a name" } else { "Rename" }, false) {
                self.alb.ppl.text = c.name.clone();
                self.alb.ppl.ask = Ask::Name;
            }
            if self.pill(ui, "Merge with…", self.alb.ppl.ask == Ask::Merge) {
                self.alb.ppl.ask = if self.alb.ppl.ask == Ask::Merge { Ask::None } else { Ask::Merge };
            }
        });
        match self.alb.ppl.ask {
            Ask::Name => {
                ui.horizontal(|ui| {
                    let r = ui.add(egui::TextEdit::singleline(&mut self.alb.ppl.text).hint_text("Name").desired_width(220.0));
                    house::target(ctx, "name field", r.rect);
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (self.pill(ui, "Save", false) || enter) && !self.alb.ppl.text.trim().is_empty() {
                        let name = self.alb.ppl.text.trim().to_string();
                        self.people_post(ctx, store, &format!("{url}/name"), json!({"name": name}));
                        if let Some(o) = self.alb.open.as_mut() {
                            o.name = name;
                        }
                        self.alb.ppl.ask = Ask::None;
                    }
                    if ui.add(link("Cancel")).clicked() {
                        self.alb.ppl.ask = Ask::None;
                    }
                });
            }
            Ask::Merge => {
                ui.label(RichText::new("The same person as:").size(house::NOTE).color(theme::text_dim()));
                let others: Vec<Card> = self.alb.ppl.named.iter().chain(&self.alb.ppl.groups).filter(|o| o.id != c.id).cloned().collect();
                ui.horizontal_wrapped(|ui| {
                    for o in others.iter().take(60) {
                        let label = if o.name.is_empty() { format!("Not named ({})", o.count) } else { o.name.clone() };
                        if self.pill(ui, &label, false) {
                            self.people_post(ctx, store, &format!("{url}/merge"), json!({"into": o.id}));
                            if let Some(op) = self.alb.open.as_mut()
                                && op.name.is_empty()
                            {
                                op.name = o.name.clone();
                            }
                            self.alb.ppl.ask = Ask::None;
                        }
                    }
                });
            }
            Ask::None => {}
        }
        // The faces: × takes a wrong one out.
        ui.add_space(house::S);
        let faces = self.alb.ppl.faces.clone();
        let mut out = None;
        ui.horizontal_wrapped(|ui| {
            for (i, f) in faces.iter().take(200).enumerate() {
                let (rect, resp) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::click());
                house::target(ctx, &format!("face:{i}"), rect);
                let p = ui.painter_at(rect);
                self.face_ui(ctx, store, &p, (&f.id, &f.v, f.bx), rect.center(), 26.0);
                if resp.hovered() {
                    p.circle_filled(rect.right_top() + vec2(-9.0, 9.0), 8.0, theme::text());
                    p.text(rect.right_top() + vec2(-9.0, 9.0), egui::Align2::CENTER_CENTER, "×", egui::FontId::proportional(13.0), Color32::WHITE);
                }
                if resp.on_hover_text("Not this person: take this face out").clicked() {
                    out = Some(f.face);
                }
            }
        });
        if let Some(face) = out {
            self.alb.ppl.faces.retain(|f| f.face != face);
            self.people_post(ctx, store, &format!("{url}/remove"), json!({"face": face}));
        }
        ui.add_space(house::S);
        if self.pics.is_empty() {
            let text = if self.loading { "Loading…" } else { "No pictures." };
            ui.label(RichText::new(text).size(house::BODY).color(theme::text_dim()));
        } else {
            self.grid(ui, store);
        }
    }
}
