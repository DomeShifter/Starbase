//! People beside the photo grid: faces grouped by the server (crates/server/src/photos/faces.rs)
//! as round crops; name a person, merge two, take a wrong face out, open a person's pictures.
//! Named people first, unnamed groups below.

use egui::{Color32, Pos2, Rect, RichText, Sense, TextureId, Ui, pos2, vec2};
use serde_json::{Value as Json, json};

use super::albums::{Card, Tab};
use super::{Got, PhotoMode, SMALL};
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
    /// The card whose name is being typed in place on the People list (its id), and whether
    /// the box still needs the keyboard.
    pub editing: Option<String>,
    focus: bool,
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
        let c = |v: &Vec<Card>| {
            v.iter()
                .map(|c| json!({"id": c.id, "name": c.name, "count": c.count, "face": c.face, "cover": c.cover.as_ref().map(|x| &x.0)}))
                .collect::<Vec<_>>()
        };
        json!({"named": c(&self.named), "groups": c(&self.groups), "on": self.state["on"], "faces": self.faces.len(), "can_download": self.state["can_download"], "download": self.state["download"], "missing": self.state["missing"].as_array().map_or(0, Vec::len), "progress": self.state["progress"], "editing": self.editing, "merging": self.ask == Ask::Merge, "scan": self.scan.test_state()})
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

    /// A round face from picture `id` (content tag `v`), box `bx`, centred at `c`. With the face's
    /// id, the server's sharp face picture (cut from the big thumbnail); while it loads, the small
    /// thumbnail when it is already here; without (or when that fails), cut from the small one.
    #[allow(clippy::too_many_arguments)]
    fn face_ui(
        &mut self,
        ctx: &egui::Context,
        store: &Backend,
        p: &egui::Painter,
        (id, v, bx): (&str, &str, [f32; 4]),
        face: Option<i64>,
        c: Pos2,
        r: f32,
    ) {
        p.circle_filled(c, r, Color32::from_rgb(0xE8, 0xEA, 0xED));
        let frame = self.frame;
        if let Some(f) = face {
            let key = format!("face/{f}/{id}");
            self.want_id(ctx, store, &key, Some(v), SMALL);
            if let Some(t) = self.tex.get(&(key.clone(), SMALL), frame) {
                round(p, t.id(), c, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)));
                return;
            }
            if !self.failed.contains(&(key, SMALL)) {
                if let Some(t) = self.tex.get(&(id.to_string(), SMALL), frame) {
                    let [w, h] = t.size();
                    round(p, t.id(), c, r, face_uv(bx, w as f32, h as f32));
                }
                return;
            }
        }
        self.want_id(ctx, store, id, Some(v), SMALL);
        if let Some(t) = self.tex.get(&(id.to_string(), SMALL), frame) {
            let [w, h] = t.size();
            round(p, t.id(), c, r, face_uv(bx, w as f32, h as f32));
        }
    }

    /// One card on the People page (144 × 184): a 120 px round face, padding [`house::M`], the
    /// name (typed in place) and the picture count [`house::XS`] apart under it.
    fn person_card(&mut self, ui: &mut Ui, ctx: &egui::Context, store: &Backend, c: &Card, n: usize) -> bool {
        let (rect, resp) = ui.allocate_exact_size(vec2(CARD_W, CARD_H), Sense::click());
        house::target(ctx, &format!("person:{n}"), rect);
        let p = ui.painter_at(rect);
        if resp.hovered() {
            p.rect_filled(rect, house::PANEL_RADIUS, theme::menu_hover());
        }
        let centre = rect.min + vec2(CARD_W / 2.0, house::M + CARD_FACE / 2.0);
        if let (Some((id, v)), Some(bx)) = (&c.cover, c.bx) {
            self.face_ui(ctx, store, &p, (id, v, bx), c.face, centre, CARD_FACE / 2.0);
        }
        if resp.hovered() {
            p.circle_stroke(centre, CARD_FACE / 2.0 + 1.0, egui::Stroke::new(3.0, theme::SHARE));
        }
        let top = house::M + CARD_FACE + house::XS;
        self.card_name(ui, ctx, store, c, n, Rect::from_min_size(rect.min + vec2(house::M, top), vec2(CARD_FACE, NAME_H)));
        let count = Rect::from_min_size(rect.min + vec2(house::M, top + NAME_H + house::XS), vec2(CARD_FACE, 14.0));
        one_line(&p, count.center_top(), &pictures(c.count), CARD_FACE, house::NOTE, theme::text_dim());
        resp.clicked()
    }

    /// The name box typed in place, for the card on the People list (`at`) or the person page's
    /// title (`at` None: laid out in the row, title size). Enter or clicking elsewhere saves (the
    /// person page's name call), Esc puts it back.
    fn name_box(&mut self, ui: &mut Ui, ctx: &egui::Context, store: &Backend, c: &Card, at: Option<Rect>, targets: &[&str]) {
        let size = if at.is_some() { house::BODY } else { house::TITLE };
        let w = at.map_or(280.0, |r| r.width());
        let edit = egui::TextEdit::singleline(&mut self.alb.ppl.text).hint_text("Name").desired_width(w).font(egui::FontId::proportional(size));
        let t = match at {
            Some(r) => ui.put(r, edit),
            None => ui.add(edit),
        };
        for name in targets {
            house::target(ctx, name, t.rect);
        }
        if std::mem::take(&mut self.alb.ppl.focus) {
            t.request_focus();
        } else if t.lost_focus() {
            let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
            self.alb.ppl.editing = None;
            if let Some(name) = name_commit(esc, &self.alb.ppl.text, &c.name) {
                self.people_post(ctx, store, &format!("api/photos/people/{}/name", c.id), json!({"name": name}));
                let open = self.alb.open.as_mut().filter(|o| o.id == c.id);
                for o in self.alb.ppl.named.iter_mut().chain(self.alb.ppl.groups.iter_mut()).chain(open).filter(|o| o.id == c.id) {
                    o.name = name.clone();
                }
            }
        }
    }

    /// Start typing a name for `c` in place.
    fn start_naming(&mut self, c: &Card) {
        self.alb.ppl.editing = Some(c.id.clone());
        self.alb.ppl.text = c.name.clone();
        self.alb.ppl.focus = true;
    }

    /// A card's name on the People list: click it (or "Add a name") to type it in place.
    fn card_name(&mut self, ui: &mut Ui, ctx: &egui::Context, store: &Backend, c: &Card, n: usize, r: Rect) {
        let target = format!("people:name:{n}");
        if self.alb.ppl.editing.as_deref() == Some(c.id.as_str()) {
            self.name_box(ui, ctx, store, c, Some(r), &[&target]);
            return;
        }
        let resp = ui.interact(r, ui.scope_id().with(("people-name", &c.id)), Sense::click()).on_hover_cursor(egui::CursorIcon::Text);
        house::target(ctx, &target, r);
        let col = if c.name.is_empty() || resp.hovered() { theme::text_dim() } else { theme::text() };
        one_line(&ui.painter_at(r), r.center_top() + vec2(0.0, 2.0), shown_name(&c.name), r.width(), house::BODY, col);
        if resp.clicked() {
            self.start_naming(c);
        }
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

    /// Turn face recognition on (fetching the models first when they are missing).
    fn turn_on(&mut self, ctx: &egui::Context, store: &Backend, st: &Json) {
        self.people_post(ctx, store, "api/photos/faces", json!({"on": true}));
        if needs_models(st) {
            self.people_post(ctx, store, "api/photos/faces/models", json!({}));
        }
    }

    /// The page title row: "People" on the left; Scan now and the one "Face recognition" menu
    /// button (on/off, download models, the privacy note) on the right.
    fn people_header(&mut self, ui: &mut Ui, ctx: &egui::Context, store: &Backend, st: &Json) {
        let on = st["on"].as_bool() == Some(true);
        ui.horizontal(|ui| {
            ui.label(RichText::new("People").size(house::TITLE).color(theme::text()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = house::S;
                if !st.is_null() && (on || st["can_toggle"].as_bool() == Some(true)) {
                    let b = ui.add(house::quiet_with(Some("expand_more"), fr_label(st), false));
                    house::target(ctx, "people:fr-menu", b.rect);
                    egui::Popup::menu(&b).show(|ui| {
                        house::menu(ui, 320.0);
                        self.fr_menu_ui(ui, ctx, store, st);
                    });
                }
                if on {
                    self.alb.ppl.scan.ui(ui, store);
                }
            });
        });
    }

    /// The rows of the face recognition menu.
    fn fr_menu_ui(&mut self, ui: &mut Ui, ctx: &egui::Context, store: &Backend, st: &Json) {
        let on = st["on"].as_bool() == Some(true);
        let busy = st["download"]["busy"].as_bool() == Some(true);
        if st["can_toggle"].as_bool() == Some(true) {
            if on {
                if house::menu_row(ui, Some("visibility_off"), "Turn off face recognition", "", true) {
                    self.people_post(ctx, store, "api/photos/faces", json!({"on": false}));
                }
            } else if !busy && house::menu_row(ui, Some("visibility"), off_view(st).2.unwrap_or("Turn on face recognition"), "", true) {
                self.turn_on(ctx, store, st);
            }
        }
        if needs_models(st) && !busy && house::menu_row(ui, Some("download"), "Download face models", "", true) {
            self.people_post(ctx, store, "api/photos/faces/models", json!({}));
        }
        house::menu_sep(ui);
        let r = egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(house::MENU_INSET, 6))
            .show(ui, |ui| {
                let dim = |ui: &mut Ui, t: &str| ui.label(RichText::new(t).size(house::NOTE).color(theme::text_dim()));
                if on {
                    let note = st["note"].as_str().unwrap_or("");
                    dim(ui, format!("{} faces found. {note}", st["progress"]["faces"].as_u64().unwrap_or(0)).trim_end());
                    ui.add_space(house::XS);
                }
                dim(ui, PRIVACY);
                if on && st["can_toggle"].as_bool() == Some(true) {
                    ui.add_space(house::XS);
                    dim(ui, "Turning off stops the scan. Names and groups are kept.");
                }
            })
            .response;
        house::target(ctx, "people:fr-note", r.rect);
    }

    /// Work in progress and problems under the title: the model download, missing model files,
    /// the scan bar. Asks the list again while something runs.
    fn people_working(&mut self, ui: &mut Ui, ctx: &egui::Context, st: &Json) {
        let on = st["on"].as_bool() == Some(true);
        self.download_ui(ui, ctx, st);
        if !on {
            return;
        }
        let missing = st["missing"].as_array().filter(|m| !m.is_empty());
        if let Some(m) = missing {
            if st["can_download"].as_bool() == Some(true) {
                if st["download"]["busy"].as_bool() != Some(true) {
                    let files: Vec<String> =
                        m.iter().map(|f| format!("{} ({})", f["what"].as_str().unwrap_or(""), f["licence"].as_str().unwrap_or(""))).collect();
                    let t = format!("Face recognition needs model files: {}. Download them from the Face recognition menu.", files.join(", "));
                    ui.label(RichText::new(t).size(house::NOTE).color(theme::text_dim()));
                }
            } else {
                let files: Vec<String> = m
                    .iter()
                    .map(|f| {
                        format!("{} ({}, {})", f["file"].as_str().unwrap_or(""), f["what"].as_str().unwrap_or(""), f["url"].as_str().unwrap_or(""))
                    })
                    .collect();
                let t =
                    format!("Model files are missing. Put them in {}: {}.", st["models"].as_str().unwrap_or("the models folder"), files.join("; "));
                ui.label(RichText::new(t).size(house::NOTE).color(house::ERROR));
            }
        }
        let scanning = self.scan_bar(ui, st, true);
        if scanning || missing.is_some() {
            self.people_poll(ui, 2.0);
        }
    }

    /// Open a person's page (from the list or the strip).
    pub(in crate::home) fn people_open(&mut self, ctx: &egui::Context, store: &Backend, c: Card) {
        self.pics.clear();
        self.viewer = None;
        self.alb.ppl.faces.clear();
        self.alb.ppl.ask = Ask::None;
        self.alb.ppl.editing = None;
        self.loading = true;
        let key = c.id.clone();
        self.alb.open = Some(c);
        super::albums::get(ctx, store, &self.inbox, &format!("api/photos/people/{key}"), move |v| Got::Opened(Tab::People, key, v));
    }

    /// The People strip at the top of Photos: a "People … See all" heading row, then round 64 px
    /// faces with their name under; a click opens the person, "See all" opens People.
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
        let mut see_all = false;
        ui.horizontal(|ui| {
            let h = ui.label(RichText::new("People").size(house::BODY).strong().color(theme::text()));
            house::target(&ctx, "people:strip-heading", h.rect);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let b = ui.add(house::quiet_with(Some("chevron_right"), "See all", false));
                house::target(&ctx, "pill-See all", b.rect);
                house::target(&ctx, "people:see-all", b.rect);
                see_all = b.clicked();
            });
        });
        ui.add_space(house::S);
        let (side, gap) = (STRIP_FACE, house::M);
        let fit = ((ui.available_width() / (side + gap)) as usize).clamp(1, 12);
        let mut open = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (n, c) in all.iter().take(fit).enumerate() {
                let (rect, resp) = ui.allocate_exact_size(vec2(side, side + house::XS + 16.0), Sense::click());
                house::target(&ctx, &format!("people-strip:{n}"), rect);
                let p = ui.painter_at(rect);
                let centre = rect.min + vec2(side / 2.0, side / 2.0);
                if let (Some((id, v)), Some(bx)) = (&c.cover, c.bx) {
                    self.face_ui(&ctx, store, &p, (id, v, bx), c.face, centre, side / 2.0 - 2.0);
                }
                if resp.hovered() {
                    p.circle_stroke(centre, side / 2.0 - 1.0, egui::Stroke::new(2.0, theme::SHARE));
                }
                let col = if c.name.is_empty() { theme::text_dim() } else { theme::text() };
                one_line(&p, rect.min + vec2(side / 2.0, side + house::XS), shown_name(&c.name), side + gap - 2.0, house::NOTE, col);
                if resp.on_hover_text(shown_name(&c.name)).clicked() {
                    open = Some(c.clone());
                }
            }
        });
        if let Some(c) = open {
            self.go(super::albums::Tab::People);
            self.people_open(&ctx, store, c);
        } else if see_all {
            self.go(super::albums::Tab::People);
        }
        ui.add_space(house::XL);
    }

    /// The model download's progress and errors (the button itself is in the face recognition
    /// menu, or the off page's main button).
    fn download_ui(&mut self, ui: &mut Ui, ctx: &egui::Context, st: &Json) {
        if st["can_download"].as_bool() != Some(true) {
            return;
        }
        let d = &st["download"];
        let mb = |b: u64| b as f64 / 1_048_576.0;
        if d["busy"].as_bool() == Some(true) {
            let (got, total) = (d["got"].as_u64().unwrap_or(0), d["total"].as_u64().unwrap_or(0).max(1));
            ui.horizontal_wrapped(|ui| {
                let r = ui.add(egui::ProgressBar::new(got as f32 / total as f32).desired_width(160.0));
                house::target(ctx, "people:download", r.rect);
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
    }

    /// The people and groups, or the opened one.
    pub(super) fn people_ui(&mut self, ui: &mut Ui, store: &Backend) {
        let ctx = ui.ctx().clone();
        self.people_boot(&ctx, store);
        if let Some(e) = self.error.clone()
            && house::status_line(ui, house::Status::Error, &e)
        {
            self.error = None;
        }
        if let Some(c) = self.alb.open.clone() {
            self.person_ui(ui, &ctx, store, &c);
            return;
        }
        let st = self.alb.ppl.state.clone();
        self.people_header(ui, &ctx, store, &st);
        ui.add_space(house::S);
        if st.is_null() {
            house::status_line(ui, house::Status::Loading, "Loading…");
            return;
        }
        self.people_working(ui, &ctx, &st);
        if st["on"].as_bool() != Some(true) {
            let (title, hint, button) = off_view(&st);
            if house::empty_state(ui, Some("groups"), title, hint, button) {
                self.turn_on(&ctx, store, &st);
            }
            return;
        }
        let (named, groups) = (self.alb.ppl.named.clone(), self.alb.ppl.groups.clone());
        if named.is_empty() && groups.is_empty() {
            let scanning = st["progress"]["done"].as_u64() < st["progress"]["all"].as_u64();
            let (title, hint) = if scanning {
                ("Still looking", "People appear here as they are found.")
            } else {
                ("No faces found", "No faces were found in these pictures.")
            };
            house::empty_state(ui, Some("groups"), title, hint, None);
            return;
        }
        let (mut open, mut n) = (None, 0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (title, list) in [("", &named), ("Unnamed groups", &groups)] {
                if list.is_empty() {
                    continue;
                }
                if !title.is_empty() {
                    ui.add_space(house::XL);
                    let h = ui.label(RichText::new(title).size(house::BODY).strong().color(theme::text()));
                    house::target(&ctx, "people:groups-heading", h.rect);
                    ui.label(RichText::new("Name these: click \"Add a name\" under a face.").size(house::NOTE).color(theme::text_dim()));
                    ui.add_space(house::M);
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

    /// A person's page: "‹ People", the name as a title you click to rename, the count, a More
    /// menu (Rename, Merge with…), the faces and then the same grid as Photos.
    fn person_ui(&mut self, ui: &mut Ui, ctx: &egui::Context, store: &Backend, c: &Card) {
        let url = format!("api/photos/people/{}", c.id);
        let back = ui.add(house::quiet_with(Some("chevron_left"), "People", false));
        house::target(ctx, "people:back", back.rect);
        if back.clicked() {
            self.alb.open = None;
            self.alb.ppl.ask = Ask::None;
            self.alb.ppl.editing = None;
            self.pics.clear();
            self.viewer = None;
            return;
        }
        ui.add_space(house::S);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = house::M;
            if self.alb.ppl.editing.as_deref() == Some(c.id.as_str()) {
                self.name_box(ui, ctx, store, c, None, &["name field", "people:title"]);
            } else {
                let col = if c.name.is_empty() { theme::text_dim() } else { theme::text() };
                let t = ui
                    .add(egui::Label::new(RichText::new(shown_name(&c.name)).size(house::TITLE).color(col)).sense(Sense::click()))
                    .on_hover_cursor(egui::CursorIcon::Text)
                    .on_hover_text("Click to rename");
                house::target(ctx, "people:title", t.rect);
                if t.clicked() {
                    self.start_naming(c);
                }
            }
            ui.label(RichText::new(pictures(c.count)).size(house::NOTE).color(theme::text_dim()));
            let more = ui.add(house::quiet_with(Some("more_vert"), "More", self.alb.ppl.ask == Ask::Merge));
            house::target(ctx, "people:more", more.rect);
            egui::Popup::menu(&more).show(|ui| {
                house::menu(ui, 220.0);
                if house::menu_row(ui, Some("edit"), if c.name.is_empty() { "Add a name" } else { "Rename" }, "", true) {
                    self.start_naming(c);
                }
                if house::menu_row(ui, Some("call_split"), "Merge with…", "", true) {
                    self.alb.ppl.ask = Ask::Merge;
                }
            });
        });
        if self.alb.ppl.ask == Ask::Merge {
            ui.add_space(house::S);
            ui.horizontal(|ui| {
                ui.label(RichText::new("The same person as:").size(house::NOTE).color(theme::text_dim()));
                if ui.add(house::quiet("Cancel")).clicked() {
                    self.alb.ppl.ask = Ask::None;
                }
            });
            let others: Vec<Card> = self.alb.ppl.named.iter().chain(&self.alb.ppl.groups).filter(|o| o.id != c.id).cloned().collect();
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(house::S, house::S);
                for o in others.iter().take(60) {
                    let label = if o.name.is_empty() { format!("Unnamed ({})", o.count) } else { o.name.clone() };
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
        // The faces: × takes a wrong one out.
        ui.add_space(house::M);
        let faces = self.alb.ppl.faces.clone();
        let mut out = None;
        ui.horizontal_wrapped(|ui| {
            for (i, f) in faces.iter().take(200).enumerate() {
                let (rect, resp) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::click());
                house::target(ctx, &format!("face:{i}"), rect);
                let p = ui.painter_at(rect);
                self.face_ui(ctx, store, &p, (&f.id, &f.v, f.bx), Some(f.face), rect.center(), 26.0);
                if resp.hovered() {
                    p.circle_filled(rect.right_top() + vec2(-9.0, 9.0), 8.0, theme::text());
                    p.text(
                        rect.right_top() + vec2(-9.0, 9.0),
                        egui::Align2::CENTER_CENTER,
                        "×",
                        egui::FontId::proportional(house::NOTE),
                        Color32::WHITE,
                    );
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
        ui.add_space(house::L);
        if self.pics.is_empty() {
            if self.loading {
                house::status_line(ui, house::Status::Loading, "Loading…");
            } else {
                house::empty_state(ui, Some("image"), "No pictures", "", None);
            }
        } else {
            self.grid(ui, store);
        }
    }
}

/// People page card: width, height, face circle (spec 3.7).
const CARD_W: f32 = 144.0;
const CARD_H: f32 = 184.0;
const CARD_FACE: f32 = 120.0;
/// The name line's height under a card's face.
const NAME_H: f32 = 18.0;
/// A face in the People strip.
const STRIP_FACE: f32 = 64.0;
/// Said wherever face recognition is offered.
const PRIVACY: &str = "It runs on this server only; nothing is sent anywhere.";

/// How a person without a name is shown, everywhere (strip, cards, person page).
fn shown_name(name: &str) -> &str {
    if name.trim().is_empty() { "Add a name" } else { name }
}

/// "1 picture", "5 pictures".
fn pictures(n: u64) -> String {
    format!("{n} {}", if n == 1 { "picture" } else { "pictures" })
}

/// The name to save after typing in place, or None (Esc, empty or unchanged).
fn name_commit(esc: bool, typed: &str, old: &str) -> Option<String> {
    let t = typed.trim();
    (!esc && !t.is_empty() && t != old).then(|| t.to_string())
}

/// True when model files are missing and this user may download them.
fn needs_models(st: &Json) -> bool {
    st["can_download"].as_bool() == Some(true) && st["missing"].as_array().is_some_and(|m| !m.is_empty())
}

/// The face recognition menu button's words.
fn fr_label(st: &Json) -> &'static str {
    if st["download"]["busy"].as_bool() == Some(true) {
        "Face recognition: downloading"
    } else if st["on"].as_bool() == Some(true) {
        "Face recognition: On"
    } else {
        "Face recognition: Off"
    }
}

/// The off page (an empty state): title, hint and the one main button (none while the models
/// download, or for someone who may not turn it on).
fn off_view(st: &Json) -> (&'static str, &'static str, Option<&'static str>) {
    const TITLE: &str = "Face recognition is off";
    if st["can_toggle"].as_bool() != Some(true) {
        return (TITLE, "Ask the owner of this server to turn it on (in Photos, People).", None);
    }
    let hint = "It groups the same face across your pictures. It runs on this server only; nothing is sent anywhere.";
    if st["download"]["busy"].as_bool() == Some(true) {
        (TITLE, hint, None)
    } else if needs_models(st) {
        (TITLE, hint, Some("Download face models and turn on"))
    } else {
        (TITLE, hint, Some("Turn on face recognition"))
    }
}

/// `text` on one line centred under `top`, cut with "…" when wider than `width`.
fn one_line(p: &egui::Painter, top: Pos2, text: &str, width: f32, size: f32, colour: Color32) {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), egui::FontId::proportional(size), colour);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    let g = p.layout_job(job);
    p.galley(top - vec2(g.size().x / 2.0, 0.0), g, colour);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unnamed_reads_add_a_name() {
        assert_eq!(shown_name(""), "Add a name");
        assert_eq!(shown_name("  "), "Add a name");
        assert_eq!(shown_name("Ada"), "Ada");
        assert_eq!(pictures(1), "1 picture");
        assert_eq!(pictures(3), "3 pictures");
    }

    #[test]
    fn typed_name_saves_only_when_new() {
        assert_eq!(name_commit(false, " Ada ", ""), Some("Ada".into()));
        assert_eq!(name_commit(true, "Ada", ""), None);
        assert_eq!(name_commit(false, "  ", "Ben"), None);
        assert_eq!(name_commit(false, "Ben", "Ben"), None);
    }

    #[test]
    fn face_menu_and_off_page() {
        let owner = json!({"on": false, "can_toggle": true, "can_download": true, "missing": [{"file": "a"}], "download": {}});
        assert_eq!(fr_label(&owner), "Face recognition: Off");
        assert_eq!(off_view(&owner).2, Some("Download face models and turn on"));
        let ready = json!({"on": false, "can_toggle": true, "can_download": true, "missing": [], "download": {}});
        assert_eq!(off_view(&ready).2, Some("Turn on face recognition"));
        let busy = json!({"on": false, "can_toggle": true, "download": {"busy": true}});
        assert_eq!(fr_label(&busy), "Face recognition: downloading");
        assert_eq!(off_view(&busy).2, None);
        let guest = json!({"on": false, "can_toggle": false});
        assert_eq!(off_view(&guest).2, None);
        assert!(off_view(&guest).1.contains("owner"));
        assert_eq!(fr_label(&json!({"on": true})), "Face recognition: On");
        assert!(off_view(&ready).1.contains(PRIVACY));
    }
}
