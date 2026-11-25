//! "View as photos": a folder of the Drive as a grid of pictures (Photos job 2), over the
//! server's photo index (crates/server/src/photos, docs/photos-api.md).
//!
//! - The switch is remembered per person (`starsheets_home_photos`). A folder where Photos isn't
//!   on yet asks first; turning it on (`POST api/photos/enable`) lets the server make thumbnails.
//! - The grid is virtualised: only the rows on screen are laid out, thumbnails are asked for in
//!   the order they're shown (plus one screen ahead), a few at a time.
//! - Textures have a budget ([`Textures`]): least recently drawn go first, never one drawn this
//!   frame. 96 MB in the browser, 256 MB on the desktop.
//! - Pictures are decoded by the browser on the web (`createImageBitmap`, off the main thread,
//!   EXIF orientation applied), by the `image` crate on the desktop.
//! - The full-screen viewer and slideshow are in viewer.rs; the editor (Edit, or E) in photo_edit.rs.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use egui::{Color32, Rect, RichText, Sense, Ui, pos2, vec2};
use serde_json::Value as Json;

mod albums;
mod people;

use super::viewer::Viewer;
use crate::house;
use crate::store::{Backend, Method, enc};
use crate::theme;

/// Thumbnail sizes the server makes; 0 = the original file.
pub(super) const SMALL: u32 = 256;
pub(super) const LARGE: u32 = 1024;
pub(super) const ORIGINAL: u32 = 0;
/// Pictures asked for at once.
const AT_ONCE: usize = 6;
/// Target size of a grid square, and the gap between squares.
const TILE: f32 = 180.0;
const GAP: f32 = 4.0;
/// How long to wait before asking again for a thumbnail the server is still making (s).
const RETRY: f64 = 2.0;
/// How often the listing is asked for again while pictures are still being read (s).
const RELIST: f64 = 3.0;

#[cfg(target_arch = "wasm32")]
const BUDGET: usize = 96 * 1024 * 1024;
#[cfg(not(target_arch = "wasm32"))]
const BUDGET: usize = 256 * 1024 * 1024;

/// A picture's texture: (file id, size).
pub(super) type Key = (String, u32);

#[derive(Clone, Debug, Default)]
pub(super) struct Pic {
    pub id: String,
    pub name: String,
    /// The content tag (thumbnail address), when the server has read it.
    pub v: Option<String>,
    pub state: String,
    pub taken: Option<String>,
    pub modified: i64,
    pub width: u32,
    pub height: u32,
}

/// Answers, from the network or a decoder (possibly another thread on the desktop).
pub(super) enum Got {
    Listed(String, bool, Json),
    Switched,
    Image(Key, Option<egui::ColorImage>),
    /// The server is still making it: ask again later.
    NotYet(Key),
    Error(String),
    /// The album or place list (albums.rs), and one opened.
    Cards(albums::Tab, Json),
    Opened(albums::Tab, String, Json),
    /// An album changed: ask again.
    Changed,
    /// A person changed (people.rs): what the opened one is called now.
    PersonIs(Option<String>),
}

/// Textures within a budget of bytes, least recently drawn going first.
#[derive(Default)]
pub(super) struct Textures {
    map: HashMap<Key, (egui::TextureHandle, usize, u64)>,
    pub total: usize,
    pub budget: usize,
}

impl Textures {
    pub fn get(&mut self, k: &Key, frame: u64) -> Option<egui::TextureHandle> {
        let e = self.map.get_mut(k)?;
        e.2 = frame;
        Some(e.0.clone())
    }
    pub fn has(&self, k: &Key) -> bool {
        self.map.contains_key(k)
    }
    #[cfg(any(test, target_arch = "wasm32"))]
    pub fn len(&self) -> usize {
        self.map.len()
    }
    /// Forgets every size of picture `id` (its edits changed).
    pub fn forget(&mut self, id: &str) {
        let gone: Vec<Key> = self.map.keys().filter(|k| k.0 == id).cloned().collect();
        for k in gone {
            if let Some(v) = self.map.remove(&k) {
                self.total -= v.1;
            }
        }
    }
    pub fn insert(&mut self, k: Key, t: egui::TextureHandle, frame: u64) {
        let bytes = t.size()[0] * t.size()[1] * 4;
        if let Some(old) = self.map.insert(k, (t, bytes, frame)) {
            self.total -= old.1;
        }
        self.total += bytes;
    }
    /// Frees textures, least recently drawn first, until within the budget; never one drawn in
    /// `frame` (it is on screen).
    pub fn evict(&mut self, frame: u64) {
        while self.total > self.budget {
            let Some(k) = self.map.iter().filter(|(_, v)| v.2 < frame).min_by_key(|(_, v)| v.2).map(|(k, _)| k.clone()) else { return };
            if let Some(v) = self.map.remove(&k) {
                self.total -= v.1;
            }
        }
    }
}

/// Columns and square size for a grid `width` wide (at least two columns).
pub(super) fn layout(width: f32) -> (usize, f32) {
    let cols = (((width + GAP) / (TILE + GAP)).floor() as usize).max(2);
    let tile = ((width - GAP * (cols as f32 - 1.0)) / cols as f32).max(40.0);
    (cols, tile)
}

/// The part of a picture `w` × `h` that fills a square (the middle of it), as texture coordinates.
pub(super) fn cover_uv(w: f32, h: f32) -> Rect {
    if w <= 0.0 || h <= 0.0 {
        return Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    }
    let a = w / h;
    if a > 1.0 {
        let cut = (1.0 - 1.0 / a) / 2.0;
        Rect::from_min_max(pos2(cut, 0.0), pos2(1.0 - cut, 1.0))
    } else {
        let cut = (1.0 - a) / 2.0;
        Rect::from_min_max(pos2(0.0, cut), pos2(1.0, 1.0 - cut))
    }
}

/// "14 July 2024" from an EXIF date (`2024-07-14T18:30:05+01:00`), or the modified time.
pub(super) fn date_label(taken: Option<&str>, modified: i64) -> String {
    const MONTHS: [&str; 12] =
        ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    if let Some(t) = taken
        && t.len() >= 10
        && let (Ok(y), Ok(m), Ok(d)) = (t[0..4].parse::<u32>(), t[5..7].parse::<usize>(), t[8..10].parse::<u32>())
        && (1..=12).contains(&m)
    {
        return format!("{d} {} {y}", MONTHS[m - 1]);
    }
    super::ago(modified)
}

pub(super) struct PhotoMode {
    /// The switch: show folders as photos.
    pub on: bool,
    booted: bool,
    /// The folder listed, and whether with its subfolders.
    pub folder: Option<String>,
    pub sub: bool,
    pub newest_first: bool,
    pub pics: Vec<Pic>,
    pub enabled: Option<bool>,
    pub can_enable: bool,
    pub can_disable: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub waiting: u64,
    listed_at: f64,
    inbox: Arc<Mutex<Vec<Got>>>,
    pub tex: Textures,
    asking: HashSet<Key>,
    retry_at: HashMap<Key, f64>,
    failed: HashSet<Key>,
    pub frame: u64,
    pub viewer: Option<Viewer>,
    /// The photo editor, over the viewer (photo_edit.rs).
    pub editor: Option<super::photo_edit::Editor>,
    /// Albums and Places beside the grid (albums.rs).
    pub alb: albums::Albums,
}

impl Default for PhotoMode {
    fn default() -> PhotoMode {
        PhotoMode {
            on: false,
            booted: false,
            folder: None,
            sub: false,
            newest_first: true,
            pics: Vec::new(),
            enabled: None,
            can_enable: false,
            can_disable: false,
            loading: false,
            error: None,
            waiting: 0,
            listed_at: -1e9,
            inbox: Arc::default(),
            tex: Textures { budget: BUDGET, ..Default::default() },
            asking: HashSet::new(),
            retry_at: HashMap::new(),
            failed: HashSet::new(),
            frame: 0,
            viewer: None,
            editor: None,
            alb: albums::Albums::default(),
        }
    }
}

impl PhotoMode {
    pub fn boot(&mut self, store: &Backend) {
        if !self.booted {
            self.booted = true;
            self.on = store.setting("starsheets_home_photos") == Some("1".into());
            self.newest_first = store.setting("starsheets_home_photos_order") != Some("old".into());
        }
    }

    pub fn set_on(&mut self, store: &Backend, on: bool) {
        self.on = on;
        store.set_setting("starsheets_home_photos", if on { "1" } else { "0" });
        if !on {
            self.viewer = None;
            self.editor = None;
        }
    }

    fn push(inbox: &Arc<Mutex<Vec<Got>>>, ctx: &egui::Context, g: Got) {
        inbox.lock().unwrap_or_else(|e| e.into_inner()).push(g);
        ctx.request_repaint();
    }

    /// Asks for the listing of `folder`.
    pub fn list(&mut self, ctx: &egui::Context, store: &Backend, folder: &str) {
        let now = ctx.input(|i| i.time);
        self.listed_at = now;
        self.loading = true;
        let order = if self.newest_first { "&desc=1" } else { "" };
        let url = format!("api/photos?folder={}&sort=taken{order}{}", enc(folder), if self.sub { "&sub=1" } else { "" });
        let (inbox, ctx2, f, sub) = (self.inbox.clone(), ctx.clone(), folder.to_string(), self.sub);
        store.call(
            ctx,
            Method::Get,
            &url,
            None,
            Box::new(move |a| {
                let g = match a {
                    Ok((200, b)) => Got::Listed(f, sub, serde_json::from_slice(&b).unwrap_or_default()),
                    Ok((s, b)) => {
                        let v: Json = serde_json::from_slice(&b).unwrap_or_default();
                        Got::Error(v["error"].as_str().map_or(format!("The server said {s}."), str::to_string))
                    }
                    Err(e) => Got::Error(e),
                };
                Self::push(&inbox, &ctx2, g);
            }),
        );
    }

    /// Turns Photos on (or off) for `folder` on the server.
    pub fn switch(&mut self, ctx: &egui::Context, store: &Backend, folder: &str, on: bool) {
        let url = format!("api/photos/{}?folder={}", if on { "enable" } else { "disable" }, enc(folder));
        let (inbox, ctx2) = (self.inbox.clone(), ctx.clone());
        store.call(
            ctx,
            Method::Post,
            &url,
            None,
            Box::new(move |a| {
                let g = match a {
                    Ok((200, _)) => Got::Switched,
                    Ok((s, b)) => {
                        let v: Json = serde_json::from_slice(&b).unwrap_or_default();
                        Got::Error(v["error"].as_str().map_or(format!("The server said {s}."), str::to_string))
                    }
                    Err(e) => Got::Error(e),
                };
                Self::push(&inbox, &ctx2, g);
            }),
        );
    }

    fn take_list(&mut self, v: &Json) {
        self.loading = false;
        self.enabled = v["enabled"].as_bool();
        self.can_enable = v["can_enable"].as_bool().unwrap_or(false);
        self.can_disable = v["can_disable"].as_bool().unwrap_or(false);
        self.waiting = v["waiting"].as_u64().unwrap_or(0);
        let pics = Self::parse_pics(v);
        self.set_pics(pics);
    }

    fn parse_pics(v: &Json) -> Vec<Pic> {
        let s = |x: &Json, k: &str| x[k].as_str().unwrap_or("").to_string();
        v["photos"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| Pic {
                id: s(p, "id"),
                name: s(p, "name"),
                v: p["v"].as_str().map(str::to_string),
                state: s(p, "state"),
                taken: p["taken"].as_str().map(str::to_string),
                modified: p["modified"].as_i64().unwrap_or(0),
                width: p["width"].as_u64().unwrap_or(0) as u32,
                height: p["height"].as_u64().unwrap_or(0) as u32,
            })
            .collect()
    }

    fn set_pics(&mut self, pics: Vec<Pic>) {
        // A picture read since: its thumbnail may be asked for again (it was "not yet").
        for p in &pics {
            if p.v.is_some() {
                self.retry_at.remove(&(p.id.clone(), SMALL));
            }
        }
        self.pics = pics;
        if let Some(vw) = &mut self.viewer {
            vw.fit_list(self.pics.len());
        }
    }

    /// Each frame: answers, and the listing again while pictures are still being read.
    pub fn logic(&mut self, ctx: &egui::Context, store: &Backend, folder: Option<String>) {
        self.frame += 1;
        let now = ctx.input(|i| i.time);
        let got: Vec<Got> = std::mem::take(&mut *self.inbox.lock().unwrap_or_else(|e| e.into_inner()));
        for g in got {
            match g {
                Got::Cards(t, v) => self.alb.take_cards(t, &v),
                Got::Opened(t, key, v) => self.take_opened(t, &key, &v),
                Got::Changed => self.alb.changed(ctx, store, &self.inbox),
                Got::PersonIs(id) => self.people_changed(ctx, store, id),
                Got::Listed(f, sub, v) => {
                    if Some(&f) == self.folder.as_ref() && sub == self.sub && self.alb.tab == albums::Tab::Photos {
                        self.take_list(&v);
                    }
                }
                Got::Switched => {
                    if let Some(f) = self.folder.clone() {
                        self.list(ctx, store, &f);
                    }
                }
                Got::Image(k, img) => {
                    self.asking.remove(&k);
                    match img {
                        Some(img) => {
                            let t = ctx.load_texture(format!("photo-{}-{}", k.0, k.1), img, egui::TextureOptions::LINEAR);
                            self.tex.insert(k, t, self.frame);
                        }
                        None => {
                            self.failed.insert(k);
                        }
                    }
                }
                Got::NotYet(k) => {
                    self.asking.remove(&k);
                    self.retry_at.insert(k, now + RETRY);
                    ctx.request_repaint_after(std::time::Duration::from_secs_f64(RETRY));
                }
                Got::Error(e) => {
                    self.loading = false;
                    self.error = Some(e);
                }
            }
        }
        if !self.on {
            return;
        }
        // Another folder (or none): start again.
        if folder != self.folder {
            self.folder = folder.clone();
            self.pics.clear();
            self.enabled = None;
            self.error = None;
            self.viewer = None;
            self.editor = None;
            self.retry_at.clear();
            self.failed.clear();
            if let Some(f) = &folder {
                self.list(ctx, store, f);
            }
        }
        if let Some(f) = &folder
            && !self.loading
            && self.enabled == Some(true)
            && self.alb.tab == albums::Tab::Photos
            && (self.waiting > 0 || self.pics.iter().any(|p| p.state == "pending"))
            && now - self.listed_at > RELIST
        {
            self.list(ctx, store, f);
        }
        if self.waiting > 0 {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(RELIST));
        }
        self.tex.evict(self.frame);
    }

    /// Asks for picture `i` at `size` if it isn't here or on its way (in the order called: the
    /// first calls of a frame are the ones on screen).
    pub fn want(&mut self, ctx: &egui::Context, store: &Backend, i: usize, size: u32) {
        let Some(p) = self.pics.get(i) else { return };
        if matches!(p.state.as_str(), "failed" | "unsupported") && size != ORIGINAL {
            return;
        }
        let (id, v) = (p.id.clone(), p.v.clone());
        self.want_id(ctx, store, &id, v.as_deref(), size);
    }

    /// The same for a picture by its id and content tag (an album's cover).
    pub fn want_id(&mut self, ctx: &egui::Context, store: &Backend, id: &str, v: Option<&str>, size: u32) {
        let k: Key = (id.to_string(), size);
        let now = ctx.input(|inp| inp.time);
        if self.tex.has(&k) || self.asking.contains(&k) || self.failed.contains(&k) || self.retry_at.get(&k).is_some_and(|t| *t > now) {
            return;
        }
        if self.asking.len() >= AT_ONCE {
            return;
        }
        let url = if size == ORIGINAL {
            format!("api/files/{id}/raw")
        } else {
            match v {
                Some(v) => format!("api/photos/thumb/{id}/{size}?v={v}"),
                None => format!("api/photos/thumb/{id}/{size}"),
            }
        };
        self.asking.insert(k.clone());
        let max = ctx.input(|inp| inp.max_texture_side).min(8192) as u32;
        let (inbox, ctx2) = (self.inbox.clone(), ctx.clone());
        store.call(
            ctx,
            Method::Get,
            &url,
            None,
            Box::new(move |a| match a {
                Ok((200, bytes)) => decode(bytes, max, k, inbox, ctx2),
                Ok((202, _)) => Self::push(&inbox, &ctx2, Got::NotYet(k)),
                _ => Self::push(&inbox, &ctx2, Got::Image(k, None)),
            }),
        );
    }

    /// Picture `id`'s edits were saved: its thumbnails are asked for again at version `v`
    /// (the server's thumbnails show the edited picture).
    pub fn edited(&mut self, id: &str, v: Option<String>) {
        self.tex.forget(id);
        for p in self.pics.iter_mut().filter(|p| p.id == id) {
            if v.is_some() {
                p.v = v.clone();
            }
        }
        self.failed.retain(|k| k.0 != id);
        self.retry_at.retain(|k, _| k.0 != id);
    }

    /// The best texture there is of picture `i`, preferring `size`: (texture, its size key).
    pub fn best(&mut self, i: usize, prefer: u32) -> Option<(egui::TextureHandle, u32)> {
        let id = self.pics.get(i)?.id.clone();
        let order: &[u32] = match prefer {
            ORIGINAL => &[ORIGINAL, LARGE, SMALL],
            LARGE => &[LARGE, ORIGINAL, SMALL],
            _ => &[SMALL, LARGE, ORIGINAL],
        };
        let frame = self.frame;
        order.iter().find_map(|&s| self.tex.get(&(id.clone(), s), frame).map(|t| (t, s)))
    }

    // ------------------------------------------------------------ drawing

    /// Left nav (Photos, Albums, Places) and what it shows.
    pub fn ui(&mut self, ui: &mut Ui, store: &Backend, folder: &str) {
        let ctx = ui.ctx().clone();
        self.alb.boot_cards(&ctx, store, &self.inbox);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(130.0);
                self.nav_ui(ui, store);
            });
            ui.separator();
            ui.vertical(|ui| match self.alb.tab {
                albums::Tab::Photos => self.folder_ui(ui, store, folder),
                albums::Tab::People => self.people_ui(ui, store),
                _ => self.cards_ui(ui, store),
            });
        });
    }

    /// The grid (or the question whether to turn Photos on) for `folder`.
    fn folder_ui(&mut self, ui: &mut Ui, store: &Backend, folder: &str) {
        if let Some(e) = self.error.clone() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(e).size(house::NOTE).color(house::ERROR));
                if ui.add(house::link("OK")).clicked() {
                    self.error = None;
                }
            });
        }
        match self.enabled {
            None => {
                ui.add_space(house::XL);
                ui.label(RichText::new(if self.loading { "Loading…" } else { "" }).size(house::BODY).color(theme::TEXT_DIM));
                return;
            }
            Some(false) => {
                self.ask_ui(ui, store, folder);
                return;
            }
            Some(true) => {}
        }
        self.bar_ui(ui, store, folder);
        self.pick_ui(ui, store);
        ui.add_space(house::S);
        if self.pics.is_empty() {
            ui.add_space(house::XL);
            let text = if self.loading { "Loading…" } else { "No pictures in this folder." };
            ui.label(RichText::new(text).size(house::BODY).color(theme::TEXT_DIM));
            return;
        }
        self.grid(ui, store);
    }

    /// The scrolling grid of `pics`.
    pub(super) fn grid(&mut self, ui: &mut Ui, store: &Backend) {
        let ctx = ui.ctx().clone();
        let (cols, tile) = layout(ui.available_width() - house::SCROLL_ROOM);
        let rows = self.pics.len().div_ceil(cols);
        ui.spacing_mut().item_spacing = vec2(GAP, GAP);
        let mut open = None;
        let shown = egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, tile, rows, |ui, range| {
            for row in range.clone() {
                ui.horizontal(|ui| {
                    for c in 0..cols {
                        let i = row * cols + c;
                        if i >= self.pics.len() {
                            break;
                        }
                        if self.tile_ui(ui, &ctx, store, i, tile) {
                            open = Some(i);
                        }
                    }
                });
            }
            range
        });
        // One screen ahead, after everything on screen was asked for.
        let range = shown.inner;
        let ahead = range.end..(range.end + range.len()).min(rows);
        let n = self.pics.len();
        for i in ahead.flat_map(|r| r * cols..((r + 1) * cols).min(n)) {
            self.want(&ctx, store, i, SMALL);
        }
        if let Some(i) = open {
            if self.alb.selecting {
                self.alb.toggle(&self.pics[i].id);
            } else {
                self.viewer = Some(Viewer::new(i, self.pics.len()));
            }
        }
    }

    /// One square of the grid; true when clicked.
    fn tile_ui(&mut self, ui: &mut Ui, ctx: &egui::Context, store: &Backend, i: usize, tile: f32) -> bool {
        let (rect, resp) = ui.allocate_exact_size(vec2(tile, tile), Sense::click());
        self.want(ctx, store, i, SMALL);
        let p = &self.pics[i];
        let (name, date, state) = (p.name.clone(), date_label(p.taken.as_deref(), p.modified), p.state.clone());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0, Color32::from_rgb(0xE8, 0xEA, 0xED));
        if let Some((t, _)) = self.best(i, SMALL) {
            let [w, h] = t.size();
            painter.image(t.id(), rect, cover_uv(w as f32, h as f32), Color32::WHITE);
        } else if state == "failed" || state == "unsupported" {
            if let Some(img) = crate::icons::icon("image") {
                img.paint_at(ui, Rect::from_center_size(rect.center() - vec2(0.0, 10.0), vec2(40.0, 40.0)));
            }
            let short: String = if name.chars().count() > 22 { name.chars().take(20).chain("…".chars()).collect() } else { name.clone() };
            painter.text(
                pos2(rect.center().x, rect.center().y + 18.0),
                egui::Align2::CENTER_TOP,
                short,
                egui::FontId::proportional(house::NOTE),
                theme::TEXT_DIM,
            );
        }
        if resp.hovered() {
            painter.rect_filled(rect, 0, Color32::from_black_alpha(40));
        }
        if self.alb.selecting {
            let on = self.alb.selected.contains(&self.pics[i].id);
            let c = rect.left_top() + vec2(16.0, 16.0);
            if on {
                painter.rect_filled(rect, 0, Color32::from_rgba_unmultiplied(0x1A, 0x73, 0xE8, 60));
            }
            painter.circle(c, 10.0, if on { super::ACCENT } else { Color32::from_white_alpha(180) }, egui::Stroke::new(1.5, Color32::WHITE));
            if on {
                painter.text(c, egui::Align2::CENTER_CENTER, "✓", egui::FontId::proportional(14.0), Color32::WHITE);
            }
        }
        resp.on_hover_text(format!("{name}\n{date}")).clicked()
    }

    /// Above the grid: how many, subfolders, order, slideshow, turning off.
    fn bar_ui(&mut self, ui: &mut Ui, store: &Backend, folder: &str) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            let n = self.pics.len();
            let mut text = format!("{n} {}", if n == 1 { "picture" } else { "pictures" });
            if self.waiting > 0 {
                text.push_str(&format!(" · preparing {}", self.waiting));
            }
            ui.label(RichText::new(text).size(house::BODY).color(theme::TEXT_DIM));
            ui.add_space(house::M);
            let pill = |ui: &mut Ui, label: &str, on: bool| {
                ui.add(
                    egui::Button::new(RichText::new(label).size(house::BODY).color(theme::TEXT))
                        .fill(if on { theme::SHARE } else { Color32::WHITE })
                        .stroke(egui::Stroke::new(1.0, theme::BOX))
                        .corner_radius(16)
                        .min_size(vec2(0.0, 32.0)),
                )
                .clicked()
            };
            if pill(ui, "Include subfolders", self.sub) {
                self.sub = !self.sub;
                self.pics.clear();
                self.list(&ctx, store, folder);
            }
            if pill(ui, if self.newest_first { "Newest first" } else { "Oldest first" }, false) {
                self.newest_first = !self.newest_first;
                store.set_setting("starsheets_home_photos_order", if self.newest_first { "new" } else { "old" });
                self.list(&ctx, store, folder);
            }
            if !self.pics.is_empty() && pill(ui, "Slideshow", false) {
                let mut v = Viewer::new(0, self.pics.len());
                v.play(ui.ctx().input(|i| i.time));
                self.viewer = Some(v);
            }
            if !self.pics.is_empty() && pill(ui, if self.alb.selecting { "Done selecting" } else { "Select" }, self.alb.selecting) {
                self.alb.selecting = !self.alb.selecting;
                self.alb.selected.clear();
            }
            if self.can_disable {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(link("Turn off Photos here")).on_hover_text("Forget this folder's thumbnails; the pictures stay as they are").clicked()
                    {
                        self.switch(&ctx, store, folder, false);
                        self.set_on(store, false);
                    }
                });
            }
        });
    }

    /// A folder where Photos isn't on yet: what it does, and the button (for people who may).
    fn ask_ui(&mut self, ui: &mut Ui, store: &Backend, folder: &str) {
        ui.add_space(house::XL);
        house::card_frame().show(ui, |ui| {
            ui.set_max_width(520.0);
            ui.label(RichText::new("Show this folder as photos?").size(house::TITLE).color(theme::TEXT));
            ui.add_space(house::S);
            house::text(
                ui,
                "Starbase makes small previews of the pictures in this folder and in the folders inside it, so they show as a grid. \
                 The pictures themselves don't change. Only people who can open this folder see the previews.",
            );
            ui.add_space(house::L);
            ui.horizontal(|ui| {
                if self.can_enable {
                    if ui.add(house::primary_in("Turn on", super::ACCENT)).clicked() {
                        let ctx = ui.ctx().clone();
                        self.switch(&ctx, store, folder, true);
                    }
                } else {
                    house::note(ui, "Ask the folder's owner to turn on Photos here.");
                }
                if ui.add(link("Show as files")).clicked() {
                    self.set_on(store, false);
                }
            });
        });
    }
}

/// The test hook's view (TESTING.md): what Photos shows, in the order shown.
#[cfg(target_arch = "wasm32")]
impl PhotoMode {
    pub(super) fn test_state(&self, shown: bool) -> Json {
        use serde_json::json;
        let has = |id: &str, s: u32| self.tex.has(&(id.to_string(), s));
        json!({
            "on": self.on,
            "shown": shown,
            "folder": self.folder,
            "enabled": self.enabled,
            "canEnable": self.can_enable,
            "canDisable": self.can_disable,
            "sub": self.sub,
            "newestFirst": self.newest_first,
            "loading": self.loading,
            "waiting": self.waiting,
            "error": self.error,
            "count": self.pics.len(),
            "albums": self.alb.test_state(),
            "people": self.alb.ppl.test_state(),
            "textures": { "count": self.tex.len(), "bytes": self.tex.total, "budget": self.tex.budget, "asking": self.asking.len() },
            "items": self.pics.iter().take(60).map(|p| json!({
                "id": p.id, "name": p.name, "taken": p.taken, "state": p.state, "v": p.v,
                "thumb": has(&p.id, SMALL), "large": has(&p.id, LARGE), "original": has(&p.id, ORIGINAL),
            })).collect::<Vec<_>>(),
            "photo_edit": self.editor.as_ref().map_or_else(|| json!({ "open": false }), |e| e.test_state()),
            "viewer": self.viewer.as_ref().map(|v| {
                let p = self.pics.get(v.index());
                json!({
                    "index": v.index(), "pos": v.pos, "id": p.map(|p| &p.id), "name": p.map(|p| &p.name),
                    "zoom": v.zoom, "playing": v.playing, "shuffle": v.shuffle, "every": v.every, "showing": v.showing,
                })
            }),
        })
    }
}

/// A text link in the Drive's blue (house::link is Sheets' green).
fn link(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).size(house::BODY).color(super::ACCENT)).frame(false)
}

/// Decodes a picture and hands the result to the inbox (the browser on the web, a thread on
/// the desktop), no bigger than `max` pixels a side.
fn decode(bytes: Vec<u8>, max: u32, k: Key, inbox: Arc<Mutex<Vec<Got>>>, ctx: egui::Context) {
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(async move {
        let img = decode_web(bytes, max).await;
        PhotoMode::push(&inbox, &ctx, Got::Image(k, img));
    });
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(move || {
        let img = decode_native(&bytes, max);
        PhotoMode::push(&inbox, &ctx, Got::Image(k, img));
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn decode_native(bytes: &[u8], max: u32) -> Option<egui::ColorImage> {
    use image::ImageDecoder;
    let mut dec = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?.into_decoder().ok()?;
    let o = dec.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(dec).ok()?;
    img.apply_orientation(o);
    if img.width().max(img.height()) > max {
        img = img.resize(max, max, image::imageops::FilterType::Triangle);
    }
    let rgba = img.into_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}

/// The browser decodes (createImageBitmap: off the main thread, EXIF orientation applied), then
/// the pixels come back through a canvas.
#[cfg(target_arch = "wasm32")]
pub(super) async fn decode_web(bytes: Vec<u8>, max: u32) -> Option<egui::ColorImage> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    let w = web_sys::window()?;
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(&bytes[..]));
    drop(bytes);
    let blob = web_sys::Blob::new_with_u8_array_sequence(&parts).ok()?;
    let mut bmp: web_sys::ImageBitmap = JsFuture::from(w.create_image_bitmap_with_blob(&blob).ok()?).await.ok()?.dyn_into().ok()?;
    let (mut bw, mut bh) = (bmp.width(), bmp.height());
    if bw.max(bh) > max {
        let s = max as f64 / bw.max(bh) as f64;
        let (nw, nh) = (((bw as f64 * s).round() as u32).max(1), ((bh as f64 * s).round() as u32).max(1));
        let opts = web_sys::ImageBitmapOptions::new();
        opts.set_resize_width(nw);
        opts.set_resize_height(nh);
        opts.set_resize_quality(web_sys::ResizeQuality::High);
        let smaller: web_sys::ImageBitmap =
            JsFuture::from(w.create_image_bitmap_with_image_bitmap_and_image_bitmap_options(&bmp, &opts).ok()?).await.ok()?.dyn_into().ok()?;
        bmp.close();
        bmp = smaller;
        (bw, bh) = (nw, nh);
    }
    let canvas = web_sys::OffscreenCanvas::new(bw, bh).ok()?;
    let cx: web_sys::OffscreenCanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
    cx.draw_image_with_image_bitmap(&bmp, 0.0, 0.0).ok()?;
    bmp.close();
    let data = cx.get_image_data(0.0, 0.0, bw as f64, bh as f64).ok()?;
    Some(egui::ColorImage::from_rgba_unmultiplied([bw as usize, bh as usize], &data.data()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_fills_the_width() {
        assert_eq!(layout(1000.0).0, 5);
        let (cols, tile) = layout(1000.0);
        assert!((cols as f32 * tile + GAP * (cols - 1) as f32 - 1000.0).abs() < 0.01);
        assert_eq!(layout(200.0).0, 2, "a phone still gets two columns");
    }

    #[test]
    fn a_square_shows_the_middle() {
        let r = cover_uv(400.0, 200.0);
        assert!((r.min.x - 0.25).abs() < 1e-6 && (r.max.x - 0.75).abs() < 1e-6 && r.min.y == 0.0 && r.max.y == 1.0);
        let r = cover_uv(100.0, 400.0);
        assert!((r.min.y - 0.375).abs() < 1e-6 && r.min.x == 0.0);
        assert_eq!(cover_uv(0.0, 0.0), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)));
    }

    #[test]
    fn dates_read_like_dates() {
        assert_eq!(date_label(Some("2024-07-14T18:30:05+01:00"), 0), "14 July 2024");
        assert_eq!(date_label(Some("2024-13-01T00:00:00"), 0), super::super::ago(0), "nonsense falls back");
    }

    #[test]
    fn textures_keep_to_their_budget() {
        let ctx = egui::Context::default();
        let mut t = Textures { budget: 3 * 16 * 16 * 4, ..Default::default() };
        let tex = |n: &str| ctx.load_texture(n, egui::ColorImage::new([16, 16], vec![Color32::RED; 256]), egui::TextureOptions::LINEAR);
        for (i, n) in ["a", "b", "c", "d"].iter().enumerate() {
            t.insert((n.to_string(), SMALL), tex(n), i as u64);
        }
        t.get(&("a".into(), SMALL), 10);
        t.evict(10);
        assert_eq!(t.len(), 3);
        assert!(t.has(&("a".into(), SMALL)), "drawn lately: kept");
        assert!(!t.has(&("b".into(), SMALL)), "the least recently drawn goes");
        // Everything drawn this frame stays, even over the budget.
        for n in ["c", "d"] {
            t.get(&(n.into(), SMALL), 11);
        }
        t.get(&("a".into(), SMALL), 11);
        t.budget = 0;
        t.evict(11);
        assert_eq!(t.len(), 3);
    }

    #[test]
    fn desktop_decoding_turns_and_shrinks() {
        let img = image::RgbImage::from_fn(40, 20, |x, _| if x < 20 { image::Rgb([255, 0, 0]) } else { image::Rgb([0, 0, 255]) });
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let c = decode_native(png.get_ref(), 10).unwrap();
        assert_eq!(c.size, [10, 5]);
        assert!(decode_native(b"not a picture", 100).is_none());
    }
}
