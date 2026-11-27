//! People beside the photo grid: faces grouped by the server (crates/server/src/photos/faces.rs)
//! as round crops; name a person, merge two, take wrong faces out (select, or right-click "Not
//! X", always confirmed), open a person's pictures. "Same person?" questions first (borderline
//! matches the server didn't join on its own), then named people, unnamed groups below, and at
//! the bottom "Ignored" (strangers) and "Not faces" (dolls): right-click a group or a face for
//! "Ignore" / "Not a face"; every one can be undone, grouped again or given to a person there.

use std::collections::{BTreeSet, HashSet};

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
    /// The group it is (or, ignored / not a face, was) in.
    pub grp: Option<i64>,
}

/// "Same person?": face `a` (alone or in a group) looks somewhat like person `person` (face `b`).
#[derive(Clone, Debug)]
pub(in crate::home) struct Question {
    pub face: i64,
    pub person: String,
    pub name: String,
    pub a: FaceCard,
    pub b: FaceCard,
}

fn face_card(f: &Json) -> Option<FaceCard> {
    let b = f["box"].as_array()?;
    let n = |i: usize| b.get(i).and_then(Json::as_f64).unwrap_or(0.0) as f32;
    Some(FaceCard {
        face: f["face"].as_i64()?,
        id: f["id"].as_str()?.to_string(),
        v: f["v"].as_str()?.to_string(),
        bx: [n(0), n(1), n(2), n(3)],
        grp: f["grp"].as_i64(),
    })
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
    /// "Same person?" questions from the list, and the ones skipped this time (by face).
    pub questions: Vec<Question>,
    skipped: HashSet<i64>,
    /// The faces ticked on a person's page, and the ones the "Remove …?" question is about.
    pub selected: BTreeSet<i64>,
    confirm: Option<Vec<i64>>,
    /// "Ignored" and "Not faces" at the bottom of the list (at most the server's 500 each).
    pub ignored: Vec<FaceCard>,
    pub notfaces: Vec<FaceCard>,
    /// The ignored or "not a face" tile (its faces) whose "Assign to" picker is open, and the
    /// tile opened to show its faces.
    assign: Option<Vec<i64>>,
    marked_open: Option<Vec<i64>>,
}

impl People {
    pub fn take(&mut self, v: &Json) {
        self.named = super::albums::cards(v, "people");
        self.groups = super::albums::cards(v, "groups");
        self.questions = v["questions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|q| {
                Some(Question {
                    face: q["face"].as_i64()?,
                    person: q["person"].as_str()?.to_string(),
                    name: q["name"].as_str().unwrap_or("").to_string(),
                    a: face_card(&q["a"])?,
                    b: face_card(&q["b"])?,
                })
            })
            .collect();
        let marks = |k: &str| v[k].as_array().into_iter().flatten().filter_map(face_card).collect::<Vec<_>>();
        self.ignored = marks("ignored");
        self.notfaces = marks("notfaces");
        // A tile changed (undone, assigned, grouped again): its picker and its open faces close.
        let tiles: Vec<Vec<i64>> =
            mark_groups(&self.ignored).into_iter().chain(mark_groups(&self.notfaces)).map(|g| g.iter().map(|f| f.face).collect()).collect();
        if self.assign.as_ref().is_some_and(|a| !tiles.contains(a)) {
            self.assign = None;
        }
        if self.marked_open.as_ref().is_some_and(|a| !tiles.contains(a)) {
            self.marked_open = None;
        }
        self.state = v.clone();
    }

    pub fn take_faces(&mut self, v: &Json) {
        self.faces = v["faces"].as_array().into_iter().flatten().filter_map(face_card).collect();
        let have: HashSet<i64> = self.faces.iter().map(|f| f.face).collect();
        self.selected.retain(|f| have.contains(f));
    }

    #[cfg(target_arch = "wasm32")]
    pub fn test_state(&self) -> Json {
        let c = |v: &Vec<Card>| {
            v.iter()
                .map(|c| json!({"id": c.id, "name": c.name, "count": c.count, "face": c.face, "cover": c.cover.as_ref().map(|x| &x.0)}))
                .collect::<Vec<_>>()
        };
        json!({"named": c(&self.named), "groups": c(&self.groups), "on": self.state["on"], "faces": self.faces.len(), "can_download": self.state["can_download"], "download": self.state["download"], "missing": self.state["missing"].as_array().map_or(0, Vec::len), "progress": self.state["progress"], "editing": self.editing, "merging": self.ask == Ask::Merge, "scan": self.scan.test_state(), "questions": self.asking().len(), "skipped": self.skipped.len(), "selected": self.selected.iter().collect::<Vec<_>>(), "confirm": self.confirm.as_ref().map(Vec::len), "ignored": self.ignored.iter().map(|f| f.face).collect::<Vec<_>>(), "notfaces": self.notfaces.iter().map(|f| f.face).collect::<Vec<_>>(), "ignored_count": self.state["ignored_count"], "notfaces_count": self.state["notfaces_count"], "assigning": self.assign, "marked_open": self.marked_open, "ignored_tiles": mark_groups(&self.ignored).iter().map(Vec::len).collect::<Vec<_>>(), "notface_tiles": mark_groups(&self.notfaces).iter().map(Vec::len).collect::<Vec<_>>()})
    }

    /// The questions not skipped, at most [`QUESTIONS`].
    fn asking(&self) -> Vec<Question> {
        self.questions.iter().filter(|q| !self.skipped.contains(&q.face)).take(QUESTIONS).cloned().collect()
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
        // No backing circle under the picture: its anti-aliased edge showed as a light ring
        // around every face (the picture's own edge isn't feathered). A placeholder only while
        // nothing is drawn, in the theme's colour.
        if !self.face_drawn(ctx, store, p, (id, v, bx), face, c, r) {
            p.circle_filled(c, r, theme::hover());
        }
    }

    /// Draws the face; false when there is nothing to draw yet.
    #[allow(clippy::too_many_arguments)]
    fn face_drawn(
        &mut self,
        ctx: &egui::Context,
        store: &Backend,
        p: &egui::Painter,
        (id, v, bx): (&str, &str, [f32; 4]),
        face: Option<i64>,
        c: Pos2,
        r: f32,
    ) -> bool {
        let frame = self.frame;
        if let Some(f) = face {
            let key = format!("face/{f}/{id}");
            self.want_id(ctx, store, &key, Some(v), SMALL);
            if let Some(t) = self.tex.get(&(key.clone(), SMALL), frame) {
                round(p, t.id(), c, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)));
                return true;
            }
            if !self.failed.contains(&(key, SMALL)) {
                if let Some(t) = self.tex.get(&(id.to_string(), SMALL), frame) {
                    let [w, h] = t.size();
                    round(p, t.id(), c, r, face_uv(bx, w as f32, h as f32));
                    return true;
                }
                return false;
            }
        }
        self.want_id(ctx, store, id, Some(v), SMALL);
        if let Some(t) = self.tex.get(&(id.to_string(), SMALL), frame) {
            let [w, h] = t.size();
            round(p, t.id(), c, r, face_uv(bx, w as f32, h as f32));
            return true;
        }
        false
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
        let mut mark = None;
        resp.context_menu(|ui| {
            house::menu(ui, 220.0);
            mark = mark_rows(ui, ctx, "Ignore group");
            house::target(ctx, "people:card-menu", ui.min_rect());
        });
        if let Some(act) = mark {
            self.people_post(ctx, store, &format!("api/photos/people/{}/{act}", c.id), json!({}));
        }
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

    /// "Finding faces: 4 of 10" as a bar; true while the scan is running. After a face finder
    /// update, older photos are looked at once more: "Checking older photos again: 4 of 10".
    fn scan_bar(&mut self, ui: &mut Ui, st: &Json, wide: bool) -> bool {
        let pr = &st["progress"];
        let n = |v: &Json| v.as_u64().unwrap_or(0);
        let (mut done, mut all, mut what) = (n(&pr["done"]), n(&pr["all"]), "Finding faces");
        if done >= all {
            (done, all, what) = (n(&pr["recheck"]["done"]), n(&pr["recheck"]["all"]), "Checking older photos again");
        }
        if done >= all {
            return false;
        }
        let bar = egui::ProgressBar::new(done as f32 / all.max(1) as f32)
            .desired_width(if wide { 320.0 } else { 220.0 })
            .text(RichText::new(format!("{what}: {done} of {all}")).size(house::NOTE));
        let r = ui.add(bar);
        house::target(ui.ctx(), "people:progress", r.rect);
        if wide {
            ui.label(RichText::new("You can keep using Photos. People appear here as they are found.").size(house::NOTE).color(theme::text_dim()));
        }
        true
    }

    /// Turn face recognition on (fetching the models first when they are missing).
    #[cfg(not(target_arch = "wasm32"))]
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
        #[cfg(target_arch = "wasm32")]
        if st["can_toggle"].as_bool() == Some(true) && house::menu_row(ui, Some("settings"), "Face recognition settings…", "", true) {
            let _ = crate::account::request(ctx, "photos-settings");
        }
        #[cfg(not(target_arch = "wasm32"))]
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
        self.alb.ppl.selected.clear();
        self.alb.ppl.confirm = None;
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
        #[cfg(target_arch = "wasm32")]
        if ctx.data_mut(|data| data.remove_temp::<bool>(egui::Id::unique(crate::account::FACES_SETTINGS_CHANGED))).unwrap_or(false) {
            self.people_changed(&ctx, store, None);
        }
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
            #[cfg(target_arch = "wasm32")]
            let button = button.map(|_| "Open face recognition settings");
            if house::empty_state(ui, Some("groups"), title, hint, button) {
                #[cfg(target_arch = "wasm32")]
                let _ = crate::account::request(&ctx, "photos-settings");
                #[cfg(not(target_arch = "wasm32"))]
                self.turn_on(&ctx, store, &st);
            }
            return;
        }
        let (named, groups) = (self.alb.ppl.named.clone(), self.alb.ppl.groups.clone());
        let marked = !self.alb.ppl.ignored.is_empty() || !self.alb.ppl.notfaces.is_empty();
        if named.is_empty() && groups.is_empty() && !marked {
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
            self.same_ui(ui, &ctx, store);
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
            self.marked_ui(ui, &ctx, store);
        });
        if let Some(c) = open {
            self.people_open(&ctx, store, c);
        }
    }

    /// "Ignored" and "Not faces" at the bottom of the list: one tile per group that was marked
    /// (its cover face and how many faces), a loose face on its own tile. A tile's buttons act on
    /// the whole group: the way back ("Stop ignoring" / "It is a face"), "Group again" (ignored
    /// only) and "Assign to…" a person. Clicking the face opens its faces, each undone alone.
    fn marked_ui(&mut self, ui: &mut Ui, ctx: &egui::Context, store: &Backend) {
        let st = self.alb.ppl.state.clone();
        let lists = [
            (true, "Ignored", "people:ignored", "ignored_count", mark_groups(&self.alb.ppl.ignored)),
            (false, "Not faces", "people:notfaces", "notfaces_count", mark_groups(&self.alb.ppl.notfaces)),
        ];
        let mut post: Option<(&str, Json)> = None;
        for (ignored, title, target, key, tiles) in lists {
            if tiles.is_empty() {
                continue;
            }
            let all: usize = tiles.iter().map(Vec::len).sum();
            ui.add_space(house::XL);
            let n_faces = st[key].as_u64().unwrap_or(all as u64);
            let h = ui.label(RichText::new(format!("{title} ({n_faces})")).size(house::BODY).strong().color(theme::text()));
            house::target(ctx, target, h.rect);
            let hint = if ignored {
                "Strangers: not shown in People, never grouped or asked about. Click a face to see its group."
            } else {
                "Not faces (dolls, posters): never found again in the same picture. Click a face to see its group."
            };
            ui.label(RichText::new(hint).size(house::NOTE).color(theme::text_dim()));
            ui.add_space(house::M);
            let (tile, back, back_act, assign) = if ignored {
                ("people:ignored-tile", "people:unignore", "unignore", "people:assign")
            } else {
                ("people:notface-tile", "people:isface", "isface", "people:nf-assign")
            };
            let back_label = if ignored { "Stop ignoring" } else { "It is a face" };
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(house::L, house::L);
                for (n, g) in tiles.iter().enumerate() {
                    let ids: Vec<i64> = g.iter().map(|f| f.face).collect();
                    let f = &g[0];
                    let rows = if ignored { 3.0 } else { 2.0 };
                    // Fixed-size tiles, top-down and centred: every tile's buttons line up.
                    let size = vec2(MARK_W, MARK_FACE + NAME_H + rows * MARK_BTN + (rows + 1.0) * house::XS);
                    ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Center), |ui| {
                        ui.set_min_size(size);
                        ui.spacing_mut().item_spacing.y = house::XS;
                        let (rect, resp) = ui.allocate_exact_size(vec2(MARK_FACE, MARK_FACE), Sense::click());
                        house::target(ctx, &format!("{tile}:{n}"), rect);
                        let p = ui.painter_at(rect);
                        self.face_ui(ctx, store, &p, (&f.id, &f.v, f.bx), Some(f.face), rect.center(), MARK_FACE / 2.0);
                        let open = self.alb.ppl.marked_open.as_ref() == Some(&ids);
                        if resp.hovered() || open {
                            p.circle_stroke(rect.center(), MARK_FACE / 2.0 - 1.0, egui::Stroke::new(3.0, theme::SHARE));
                        }
                        if resp.on_hover_text("Show the faces in this group").clicked() {
                            self.alb.ppl.marked_open = if open { None } else { Some(ids.clone()) };
                        }
                        let count = if g.len() == 1 { "1 face".to_string() } else { format!("{} faces", g.len()) };
                        ui.add_sized(vec2(MARK_W, NAME_H), egui::Label::new(RichText::new(count).size(house::NOTE).color(theme::text_dim())));
                        let button = |ui: &mut Ui, label: &str, name: String| {
                            let r = ui.add_sized(vec2(MARK_W, MARK_BTN), house::quiet(label));
                            house::target(ctx, &name, r.rect);
                            r.clicked()
                        };
                        if button(ui, back_label, format!("{back}:{n}")) {
                            post = Some((back_act, json!({"faces": ids})));
                        }
                        if ignored && button(ui, "Group again", format!("people:regroup:{n}")) {
                            post = Some(("regroup", json!({"faces": ids})));
                        }
                        if button(ui, "Assign to…", format!("{assign}:{n}")) {
                            self.alb.ppl.assign = if self.alb.ppl.assign.as_ref() == Some(&ids) { None } else { Some(ids.clone()) };
                        }
                    });
                }
            });
            // The faces of the opened tile, each with its own way back.
            let opened =
                self.alb.ppl.marked_open.clone().and_then(|o| tiles.iter().find(|g| g.iter().map(|f| f.face).eq(o.iter().copied())).cloned());
            if let Some(faces) = opened {
                ui.add_space(house::S);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(house::M, house::M);
                    for (i, f) in faces.iter().enumerate() {
                        let size = vec2(MARK_W, 48.0 + MARK_BTN + house::XS);
                        ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Center), |ui| {
                            ui.set_min_size(size);
                            ui.spacing_mut().item_spacing.y = house::XS;
                            let (rect, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
                            house::target(ctx, &format!("people:marked-face:{i}"), rect);
                            let p = ui.painter_at(rect);
                            self.face_ui(ctx, store, &p, (&f.id, &f.v, f.bx), Some(f.face), rect.center(), 24.0);
                            let r = ui.add_sized(vec2(MARK_W, MARK_BTN), house::quiet(back_label));
                            house::target(ctx, &format!("people:marked-undo:{i}"), r.rect);
                            if r.clicked() {
                                post = Some((back_act, json!({"faces": [f.face]})));
                            }
                        });
                    }
                });
            }
            if let Some(ids) = self.alb.ppl.assign.clone().filter(|a| tiles.iter().flatten().any(|f| a.contains(&f.face))) {
                ui.add_space(house::S);
                ui.label(RichText::new("Assign to:").size(house::NOTE).color(theme::text_dim()));
                let others: Vec<Card> = self.alb.ppl.named.iter().chain(&self.alb.ppl.groups).cloned().collect();
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(house::S, house::S);
                    for (i, o) in others.iter().take(60).enumerate() {
                        let label = if o.name.is_empty() { format!("Unnamed ({})", o.count) } else { o.name.clone() };
                        let r = ui.add(house::secondary_in(&label, theme::BLUE));
                        house::target(ctx, &format!("people:assign-to:{i}"), r.rect);
                        if r.clicked() {
                            post = Some(("assign", json!({"faces": ids, "into": o.id})));
                        }
                    }
                    let x = ui.add(house::quiet("Cancel"));
                    house::target(ctx, "people:assign-cancel", x.rect);
                    if x.clicked() {
                        self.alb.ppl.assign = None;
                    }
                });
            }
        }
        if let Some((act, body)) = post {
            self.alb.ppl.assign = None;
            self.people_post(ctx, store, &format!("api/photos/people/marked/{act}"), body);
        }
    }

    /// Back from a person's page to the list.
    fn person_close(&mut self) {
        self.alb.open = None;
        self.alb.ppl.ask = Ask::None;
        self.alb.ppl.editing = None;
        self.alb.ppl.selected.clear();
        self.alb.ppl.confirm = None;
        self.pics.clear();
        self.viewer = None;
    }

    /// "Same person?" on the People list: the borderline face beside the person's, Yes / No /
    /// Skip. Yes joins them, No is remembered (never asked again), Skip hides it until reload.
    fn same_ui(&mut self, ui: &mut Ui, ctx: &egui::Context, store: &Backend) {
        let qs = self.alb.ppl.asking();
        if qs.is_empty() {
            return;
        }
        let h = ui.label(RichText::new("Same person?").size(house::BODY).strong().color(theme::text()));
        house::target(ctx, "people:same-heading", h.rect);
        ui.label(RichText::new("These look alike, but not enough to be sure.").size(house::NOTE).color(theme::text_dim()));
        ui.add_space(house::M);
        let mut answer = None;
        for (k, q) in qs.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = house::M;
                let (r, _) = ui.allocate_exact_size(vec2(2.0 * SAME_FACE + house::S, SAME_FACE), Sense::hover());
                house::target(ctx, &format!("people:same-faces:{k}"), r);
                let p = ui.painter_at(r);
                let rad = SAME_FACE / 2.0;
                self.face_ui(ctx, store, &p, (&q.a.id, &q.a.v, q.a.bx), Some(q.a.face), r.left_center() + vec2(rad, 0.0), rad);
                self.face_ui(ctx, store, &p, (&q.b.id, &q.b.v, q.b.bx), Some(q.b.face), r.right_center() - vec2(rad, 0.0), rad);
                ui.label(RichText::new(format!("Is this {}?", who_label(&q.name))).size(house::BODY).color(theme::text()));
                for (label, act, button) in
                    [("Yes", Some(true), house::primary("Yes")), ("No", Some(false), house::secondary("No")), ("Skip", None, house::quiet("Skip"))]
                {
                    let b = ui.add(button);
                    let name = format!("people:same-{}", label.to_lowercase());
                    house::target(ctx, &format!("{name}:{k}"), b.rect);
                    if k == 0 {
                        house::target(ctx, &name, b.rect);
                    }
                    if b.clicked() {
                        answer = Some((q.clone(), act));
                    }
                }
            });
            ui.add_space(house::S);
        }
        match answer {
            Some((q, Some(yes))) => {
                self.alb.ppl.questions.retain(|o| o.face != q.face);
                self.people_post(ctx, store, &format!("api/photos/people/{}/same", q.person), json!({"face": q.face, "yes": yes}));
            }
            Some((q, None)) => {
                self.alb.ppl.skipped.insert(q.face);
            }
            None => {}
        }
        ui.add_space(house::L);
    }

    /// The "Remove N faces from X?" question; Remove sends them back to the unnamed groups.
    fn remove_dialog(&mut self, ctx: &egui::Context, store: &Backend, c: &Card) {
        let Some(ids) = self.alb.ppl.confirm.clone() else { return };
        let n = ids.len();
        let title = format!("Remove {n} {} from {}?", if n == 1 { "face" } else { "faces" }, who_label(&c.name));
        let (mut close, mut yes, mut cancel) = (false, false, false);
        house::dialog(ctx, "people_remove_confirm", 380.0, &title, Some(&mut close), |ui| {
            let body = if c.id.starts_with('p') {
                if n == 1 { "It will go back to Unnamed." } else { "They will go back to Unnamed." }
            } else {
                "They will be taken out of this group."
            };
            house::note(ui, body);
            ui.add_space(house::L);
            ui.horizontal(|ui| {
                let y = ui.add(house::primary("Remove"));
                house::target(ctx, "people:remove-yes", y.rect);
                yes = y.clicked();
                let no = ui.add(house::secondary("Cancel"));
                house::target(ctx, "people:remove-cancel", no.rect);
                cancel = no.clicked();
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        if yes {
            self.alb.ppl.confirm = None;
            self.alb.ppl.selected.clear();
            self.alb.ppl.faces.retain(|f| !ids.contains(&f.face));
            self.people_post(ctx, store, &format!("api/photos/people/{}/remove", c.id), json!({"faces": ids}));
            if self.alb.ppl.faces.is_empty() {
                self.person_close();
                self.alb.ppl.asked = false;
            }
        } else if close || cancel {
            self.alb.ppl.confirm = None;
        }
    }

    /// A person's faces: click opens the picture, the circle at the top left selects, right-click
    /// offers "Not X". Every removal is asked first ([`Self::remove_dialog`]).
    fn faces_grid(&mut self, ui: &mut Ui, ctx: &egui::Context, store: &Backend, c: &Card) {
        let label = who_label(&c.name).to_string();
        let n = self.alb.ppl.selected.len();
        if n > 0 {
            ui.horizontal(|ui| {
                ui.set_min_height(36.0);
                ui.spacing_mut().item_spacing.x = house::M;
                ui.label(RichText::new(format!("{n} selected")).size(house::BODY).strong().color(theme::text()));
                let r = ui.add(house::primary(&format!("Remove {n} from {label}")));
                house::target(ctx, "people:remove-selected", r.rect);
                if r.clicked() {
                    self.alb.ppl.confirm = Some(self.alb.ppl.selected.iter().copied().collect());
                }
                let x = ui.add(house::quiet("Cancel"));
                house::target(ctx, "people:select-cancel", x.rect);
                if x.clicked() {
                    self.alb.ppl.selected.clear();
                }
            });
            ui.add_space(house::S);
        }
        let faces = self.alb.ppl.faces.clone();
        let (mut open, mut toggle, mut ask, mut mark) = (None, None, None, None);
        let any = n > 0;
        ui.horizontal_wrapped(|ui| {
            for (i, f) in faces.iter().take(200).enumerate() {
                let (rect, resp) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::click());
                house::target(ctx, &format!("face:{i}"), rect);
                let dot = Rect::from_center_size(rect.left_top() + vec2(10.0, 10.0), vec2(20.0, 20.0));
                house::target(ctx, &format!("people:face-select:{i}"), dot);
                let p = ui.painter_at(rect);
                self.face_ui(ctx, store, &p, (&f.id, &f.v, f.bx), Some(f.face), rect.center(), 26.0);
                let on = self.alb.ppl.selected.contains(&f.face);
                if on {
                    p.circle_stroke(rect.center(), 26.0, egui::Stroke::new(3.0, super::super::ACCENT));
                }
                if on || any || resp.hovered() {
                    let fill = if on { super::super::ACCENT } else { Color32::from_black_alpha(90) };
                    p.circle(dot.center(), 8.0, fill, egui::Stroke::new(1.5, Color32::WHITE));
                    if on {
                        p.text(dot.center(), egui::Align2::CENTER_CENTER, "✓", egui::FontId::proportional(11.0), Color32::WHITE);
                    }
                }
                let resp = resp.on_hover_text("Click to open; the circle selects; right-click for more");
                if resp.clicked() {
                    if resp.interact_pointer_pos().is_some_and(|q| dot.contains(q)) {
                        toggle = Some(f.face);
                    } else {
                        open = Some(f.id.clone());
                    }
                }
                resp.context_menu(|ui| {
                    house::menu(ui, 220.0);
                    if house::menu_row(ui, Some("close"), &format!("Not {label}"), "", true) {
                        ask = Some(vec![f.face]);
                    }
                    if let Some(act) = mark_rows(ui, ctx, "Ignore face") {
                        mark = Some((act, f.face));
                    }
                    house::target(ctx, "people:face-menu", ui.min_rect());
                });
            }
        });
        if let Some(f) = toggle
            && !self.alb.ppl.selected.remove(&f)
        {
            self.alb.ppl.selected.insert(f);
        }
        if let Some(ids) = ask {
            self.alb.ppl.confirm = Some(ids);
        }
        if let Some((act, face)) = mark {
            self.alb.ppl.selected.remove(&face);
            // The page must not keep offering a face that has left People (a later "Not a face"
            // for the page's faces would be refused as "not in People").
            self.alb.ppl.faces.retain(|f| f.face != face);
            self.people_post(ctx, store, &format!("api/photos/people/faces/{act}"), json!({"faces": [face]}));
        }
        if let Some(id) = open
            && let Some(i) = self.pics.iter().position(|p| p.id == id)
        {
            self.viewer = Some(super::Viewer::new(i, self.pics.len()));
        }
        self.remove_dialog(ctx, store, c);
    }

    /// A person's page: "‹ People", the name as a title you click to rename, the count, a More
    /// menu (Rename, Merge with…), the faces and then the same grid as Photos.
    fn person_ui(&mut self, ui: &mut Ui, ctx: &egui::Context, store: &Backend, c: &Card) {
        let url = format!("api/photos/people/{}", c.id);
        let back = ui.add(house::quiet_with(Some("chevron_left"), "People", false));
        house::target(ctx, "people:back", back.rect);
        if back.clicked() {
            self.person_close();
            return;
        }
        ui.add_space(house::S);
        let mut gone = None;
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
                ui.separator();
                gone = mark_rows(ui, ctx, "Ignore group");
            });
        });
        if let Some(act) = gone {
            // The group leaves People: back to the list, then mark it.
            self.person_close();
            // By the page's faces: after one face left, the group itself may already be gone
            // (regrouped), so its id would no longer be found (verify of #183).
            let ids: Vec<i64> = self.alb.ppl.faces.iter().map(|f| f.face).collect();
            if ids.is_empty() {
                self.people_post(ctx, store, &format!("{url}/{act}"), json!({}));
            } else {
                self.people_post(ctx, store, &format!("api/photos/people/faces/{act}"), json!({"faces": ids}));
            }
            return;
        }
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
        ui.add_space(house::M);
        self.faces_grid(ui, ctx, store, c);
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
/// A face in a "Same person?" question.
const SAME_FACE: f32 = 64.0;
/// The most "Same person?" questions shown at once.
const QUESTIONS: usize = 5;
/// A face in "Ignored" / "Not faces", and the room for its buttons.
const MARK_FACE: f32 = 64.0;
const MARK_W: f32 = 128.0;
/// A tile button's height.
const MARK_BTN: f32 = 28.0;
/// Said wherever face recognition is offered.
const PRIVACY: &str = "It runs on this server only; nothing is sent anywhere.";

/// "Ignore …" and "Not a face" in a right-click or More menu: the server action picked.
/// Ignored or "not a face" faces as tiles: one per group they were in (in list order), a face
/// without a group alone.
fn mark_groups(list: &[FaceCard]) -> Vec<Vec<FaceCard>> {
    let mut out: Vec<Vec<FaceCard>> = Vec::new();
    let mut at: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    for f in list {
        match f.grp.and_then(|g| at.get(&g).copied()) {
            Some(i) => out[i].push(f.clone()),
            None => {
                if let Some(g) = f.grp {
                    at.insert(g, out.len());
                }
                out.push(vec![f.clone()]);
            }
        }
    }
    out
}

fn mark_rows(ui: &mut Ui, ctx: &egui::Context, ignore: &str) -> Option<&'static str> {
    let mut out = None;
    for (icon, label, act, target) in
        [("visibility_off", ignore, "ignore", "people:ctx-ignore"), ("close", "Not a face", "notface", "people:ctx-notface")]
    {
        let y = ui.cursor().min.y;
        if house::menu_row(ui, Some(icon), label, "", true) {
            out = Some(act);
        }
        let row = Rect::from_min_max(pos2(ui.max_rect().min.x, y), pos2(ui.max_rect().max.x, ui.cursor().min.y.max(y + 32.0)));
        house::target(ctx, target, row);
    }
    out
}

/// How a person without a name is shown, everywhere (cards, person page).
fn shown_name(name: &str) -> &str {
    if name.trim().is_empty() { "Add a name" } else { name }
}

/// A person or group in a sentence ("Remove 2 from Ann", "Is this Ann?").
fn who_label(name: &str) -> &str {
    if name.trim().is_empty() { "Unnamed" } else { name }
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
        return (TITLE, "Ask the owner of this server to turn it on.", None);
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

    #[test]
    fn ignored_and_not_faces_are_read_from_the_list() {
        let face = |n: i64, g: Option<i64>| json!({"face": n, "id": "f1", "v": "t", "box": [0.1, 0.1, 0.2, 0.2], "grp": g});
        let mut p = People { assign: Some(vec![7, 8]), marked_open: Some(vec![7, 8]), ..Default::default() };
        p.take(&json!({"ignored": [face(7, Some(3)), face(8, Some(3))], "notfaces": [face(9, None)], "ignored_count": 2, "notfaces_count": 1}));
        assert_eq!(p.ignored.iter().map(|f| f.face).collect::<Vec<_>>(), [7, 8]);
        assert_eq!(p.notfaces[0].face, 9);
        assert_eq!(p.assign, Some(vec![7, 8]), "still listed: the picker stays open");
        p.take(&json!({"ignored": [face(7, Some(3))], "notfaces": [face(9, None)]}));
        assert_eq!((p.assign.clone(), p.marked_open.clone()), (None, None), "the tile changed: picker and faces close");
    }

    /// Verify of #183: an ignored group of 7 is one tile, not 7; loose faces are a tile each.
    #[test]
    fn marked_faces_are_one_tile_per_group() {
        let f = |face: i64, grp: Option<i64>| FaceCard { face, id: "x".into(), v: "v".into(), bx: [0.0; 4], grp };
        let list: Vec<FaceCard> = (1..=7).map(|i| f(i, Some(1))).chain([f(20, None), f(21, Some(21)), f(30, None)]).chain([f(8, Some(1))]).collect();
        let tiles = mark_groups(&list);
        assert_eq!(tiles.iter().map(Vec::len).collect::<Vec<_>>(), [8, 1, 1, 1]);
        assert_eq!(tiles[0].iter().map(|x| x.face).collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(mark_groups(&[]).is_empty());
    }
}
