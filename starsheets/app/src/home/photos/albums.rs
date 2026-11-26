//! Albums and Places beside the photo grid: the left nav (Photos, Albums, Places), the album and
//! place cards, an opened album or place (the same grid), and picking pictures to add to an
//! album (selecting itself, the menu and the album picker are pick.rs). The server side is `crates/server/src/photos/albums.rs`.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use egui::{Color32, RichText, Sense, Ui, vec2};
use serde_json::{Value as Json, json};

use super::{Got, PhotoMode, SMALL, link};
use crate::house;
use crate::store::{Backend, Method, enc};
use crate::theme;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::home) enum Tab {
    #[default]
    Photos,
    Albums,
    Places,
    Events,
    Duplicates,
}

impl Tab {
    #[cfg(target_arch = "wasm32")]
    fn name(self) -> &'static str {
        match self {
            Tab::Photos => "photos",
            Tab::Albums => "albums",
            Tab::Places => "places",
            Tab::Events => "events",
            Tab::Duplicates => "duplicates",
        }
    }
}

/// An album or a place on its card.
#[derive(Clone, Debug, Default)]
pub(in crate::home) struct Card {
    /// The album's id; a place's name.
    pub id: String,
    pub name: String,
    pub count: u64,
    /// The cover picture's id and content tag.
    pub cover: Option<(String, String)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Ask {
    #[default]
    None,
    New,
    Rename,
    Delete,
}

#[derive(Default)]
pub(in crate::home) struct Albums {
    pub tab: Tab,
    pub(super) albums: Vec<Card>,
    places: Vec<Card>,
    events: Vec<Card>,
    asked: bool,
    asked_places: bool,
    asked_events: bool,
    /// The album or place opened.
    pub open: Option<Card>,
    pub selected: HashSet<String>,
    text: String,
    ask: Ask,
}

fn cards(v: &Json, key: &str) -> Vec<Card> {
    v[key]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| Card {
            id: c["id"].as_str().or(c["name"].as_str()).unwrap_or("").to_string(),
            name: c["name"].as_str().unwrap_or("").to_string(),
            count: c["count"].as_u64().unwrap_or(0),
            cover: c["cover"]["id"].as_str().zip(c["cover"]["v"].as_str()).map(|(a, b)| (a.to_string(), b.to_string())),
        })
        .collect()
}

type Inbox = Arc<Mutex<Vec<Got>>>;

pub(super) fn get(ctx: &egui::Context, store: &Backend, inbox: &Inbox, url: &str, wrap: impl FnOnce(Json) -> Got + 'static) {
    let (inbox, ctx2) = (inbox.clone(), ctx.clone());
    store.call(
        ctx,
        Method::Get,
        url,
        None,
        Box::new(move |a| {
            let g = match a {
                Ok((200, b)) => wrap(serde_json::from_slice(&b).unwrap_or_default()),
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

pub(super) fn post(ctx: &egui::Context, store: &Backend, inbox: &Inbox, url: &str, body: Json) {
    let (inbox, ctx2) = (inbox.clone(), ctx.clone());
    store.call(
        ctx,
        Method::Post,
        url,
        Some(body.to_string().into_bytes()),
        Box::new(move |a| {
            let g = match a {
                Ok((200, _)) => Got::Changed,
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

impl Albums {
    /// Asks for the album list once (the "Add to album" choices need it).
    pub fn boot_cards(&mut self, ctx: &egui::Context, store: &Backend, inbox: &Inbox) {
        if !self.asked {
            self.asked = true;
            get(ctx, store, inbox, "api/photos/albums", |v| Got::Cards(Tab::Albums, v));
        }
        if self.tab == Tab::Places && !self.asked_places {
            self.asked_places = true;
            get(ctx, store, inbox, "api/photos/places", |v| Got::Cards(Tab::Places, v));
        }
        if self.tab == Tab::Events && !self.asked_events {
            self.asked_events = true;
            get(ctx, store, inbox, "api/photos/events", |v| Got::Cards(Tab::Events, v));
        }
    }

    pub fn take_cards(&mut self, t: Tab, v: &Json) {
        match t {
            Tab::Albums => self.albums = cards(v, "albums"),
            Tab::Places => self.places = cards(v, "places"),
            Tab::Events => self.events = cards(v, "events"),
            Tab::Photos | Tab::Duplicates => {}
        }
    }

    /// An album changed on the server: its list (and the opened album) are asked for again.
    pub fn changed(&mut self, ctx: &egui::Context, store: &Backend, inbox: &Inbox) {
        get(ctx, store, inbox, "api/photos/albums", |v| Got::Cards(Tab::Albums, v));
        if let Some(c) = self.open.clone()
            && self.tab == Tab::Albums
        {
            get(ctx, store, inbox, &format!("api/photos/albums/{}", c.id), move |v| Got::Opened(Tab::Albums, c.id, v));
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn test_state(&self) -> Json {
        let c = |v: &Vec<Card>| {
            v.iter().map(|c| json!({"id": c.id, "name": c.name, "count": c.count, "cover": c.cover.as_ref().map(|c| &c.0)})).collect::<Vec<_>>()
        };
        let mut sel: Vec<&String> = self.selected.iter().collect();
        sel.sort();
        json!({
            "tab": self.tab.name(), "albums": c(&self.albums), "places": c(&self.places), "events": c(&self.events),
            "open": self.open.as_ref().map(|c| json!({"id": c.id, "name": c.name})),
            "selecting": !self.selected.is_empty(), "selected": sel,
        })
    }
}

impl PhotoMode {
    pub(super) fn go(&mut self, t: Tab) {
        if self.alb.tab != t {
            self.alb.tab = t;
            self.alb.open = None;
            self.clear_selection();
            self.alb.ask = Ask::None;
            self.dups.reset();
            self.pics.clear();
            self.viewer = None;
            // Back to the folder: listed again.
            if t == Tab::Photos {
                self.folder = None;
            }
        }
    }

    /// Left nav.
    pub(super) fn nav_ui(&mut self, ui: &mut Ui, _store: &Backend) {
        ui.add_space(house::S);
        for (t, label, icon) in [
            (Tab::Photos, "Photos", "photo_library"),
            (Tab::Albums, "Albums", "collections"),
            (Tab::Places, "Places", "location_on"),
            (Tab::Events, "Events", "event"),
            (Tab::Duplicates, "Duplicates", "content_copy"),
        ] {
            let on = self.alb.tab == t;
            let b = egui::Button::new(RichText::new(format!("  {label}")).size(house::BODY).color(theme::TEXT))
                .fill(if on { theme::SHARE } else { Color32::TRANSPARENT })
                .corner_radius(16)
                .min_size(vec2(120.0, 34.0));
            let b = match crate::icons::icon(icon) {
                Some(img) => {
                    egui::Button::image_and_text(img.fit_to_exact_size(vec2(18.0, 18.0)), RichText::new(label).size(house::BODY).color(theme::TEXT))
                        .fill(if on { theme::SHARE } else { Color32::TRANSPARENT })
                        .corner_radius(16)
                        .min_size(vec2(120.0, 34.0))
                }
                None => b,
            };
            if ui.add(b).clicked() {
                self.go(t);
            }
        }
    }

    pub(super) fn take_opened(&mut self, t: Tab, key: &str, v: &Json) {
        if self.alb.tab != t || self.alb.open.as_ref().map(|c| c.id.as_str()) != Some(key) {
            return;
        }
        self.loading = false;
        self.waiting = v["waiting"].as_u64().unwrap_or(0);
        let pics = Self::parse_pics(v);
        self.alb.selected.retain(|id| pics.iter().any(|p| &p.id == id));
        self.set_pics(pics);
    }

    pub(super) fn open_card(&mut self, ctx: &egui::Context, store: &Backend, c: Card) {
        self.pics.clear();
        self.viewer = None;
        self.clear_selection();
        self.alb.ask = Ask::None;
        self.loading = true;
        let t = self.alb.tab;
        let url = match t {
            Tab::Albums => format!("api/photos/albums/{}", c.id),
            Tab::Events => format!("api/photos/events?event={}", enc(&c.id)),
            _ => format!("api/photos/places?place={}", enc(&c.id)),
        };
        let key = c.id.clone();
        self.alb.open = Some(c);
        get(ctx, store, &self.inbox, &url, move |v| Got::Opened(t, key, v));
    }

    /// The album and place cards, or the opened one.
    pub(super) fn cards_ui(&mut self, ui: &mut Ui, store: &Backend) {
        let ctx = ui.ctx().clone();
        if let Some(e) = self.error.clone() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(e).size(house::NOTE).color(house::ERROR));
                if ui.add(house::link("OK")).clicked() {
                    self.error = None;
                }
            });
        }
        let t = self.alb.tab;
        if let Some(c) = self.alb.open.clone() {
            if !self.alb.selected.is_empty() {
                self.sel_bar_ui(ui, store);
            } else {
                ui.horizontal(|ui| {
                    if ui
                        .add(link(match t {
                            Tab::Albums => "‹ Albums",
                            Tab::Events => "‹ Events",
                            _ => "‹ Places",
                        }))
                        .clicked()
                    {
                        self.alb.open = None;
                        self.clear_selection();
                        self.alb.ask = Ask::None;
                        self.pics.clear();
                        self.viewer = None;
                    }
                    ui.label(RichText::new(&c.name).size(house::TITLE).color(theme::TEXT));
                    let n = self.pics.len();
                    ui.label(RichText::new(format!("{n} {}", if n == 1 { "picture" } else { "pictures" })).size(house::BODY).color(theme::TEXT_DIM));
                    if t == Tab::Events && !self.pics.is_empty() && self.pill(ui, "Save as album", false) {
                        self.create(ui.ctx(), store, &c.name, true);
                    }
                    if t == Tab::Albums {
                        if self.pill(ui, "Add photos", false) {
                            self.open_picker(ui.ctx(), store);
                        }
                        if self.pill(ui, "Rename", false) {
                            self.alb.text = c.name.clone();
                            self.alb.ask = Ask::Rename;
                        }
                        if self.pill(ui, "Delete album", false) {
                            self.alb.ask = Ask::Delete;
                        }
                    }
                });
            }
            self.pick_ui(ui, store);
            ui.add_space(house::S);
            if self.pics.is_empty() {
                ui.add_space(house::XL);
                let text = if self.loading {
                    "Loading…"
                } else if t == Tab::Albums {
                    "No pictures here yet. Click Add photos, or right-click pictures in Photos and choose Add to album."
                } else {
                    "No pictures."
                };
                ui.label(RichText::new(text).size(house::BODY).color(theme::TEXT_DIM));
            } else {
                self.grid(ui, store);
            }
            return;
        }
        if t == Tab::Albums {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Albums").size(house::TITLE).color(theme::TEXT));
                if self.pill(ui, "New album", false) {
                    self.alb.text.clear();
                    self.alb.ask = Ask::New;
                }
            });
            self.pick_ui(ui, store);
        } else {
            ui.label(RichText::new(if t == Tab::Events { "Events" } else { "Places" }).size(house::TITLE).color(theme::TEXT));
            self.alb.boot_cards(&ctx, store, &self.inbox);
        }
        ui.add_space(house::S);
        let list = match t {
            Tab::Albums => self.alb.albums.clone(),
            Tab::Events => self.alb.events.clone(),
            _ => self.alb.places.clone(),
        };
        if list.is_empty() {
            ui.add_space(house::XL);
            let text = if t == Tab::Albums {
                "No albums yet."
            } else if t == Tab::Events {
                "No events yet: five or more pictures taken close together in time show up here."
            } else {
                "No places yet: pictures with a position (GPS) from folders you can edit show up here."
            };
            ui.label(RichText::new(text).size(house::BODY).color(theme::TEXT_DIM));
            return;
        }
        let mut open = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(house::L, house::L);
                for c in &list {
                    if self.card_ui(ui, &ctx, store, c) {
                        open = Some(c.clone());
                    }
                }
            });
        });
        if let Some(c) = open {
            self.open_card(&ctx, store, c);
        }
    }

    fn pill(&self, ui: &mut Ui, label: &str, on: bool) -> bool {
        ui.add(
            egui::Button::new(RichText::new(label).size(house::BODY).color(theme::TEXT))
                .fill(if on { theme::SHARE } else { Color32::WHITE })
                .stroke(egui::Stroke::new(1.0, theme::BOX))
                .corner_radius(16)
                .min_size(vec2(0.0, 32.0)),
        )
        .clicked()
    }

    fn card_ui(&mut self, ui: &mut Ui, ctx: &egui::Context, store: &Backend, c: &Card) -> bool {
        let side = 176.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(side, side + 40.0), Sense::click());
        let img = egui::Rect::from_min_size(rect.min, vec2(side, side));
        let p = ui.painter_at(rect);
        p.rect_filled(img, 8, Color32::from_rgb(0xE8, 0xEA, 0xED));
        if let Some((id, v)) = &c.cover {
            self.want_id(ctx, store, id, Some(v), SMALL);
            let frame = self.frame;
            if let Some(t) = self.tex.get(&(id.clone(), SMALL), frame) {
                let [w, h] = t.size();
                p.image(t.id(), img, super::cover_uv(w as f32, h as f32), Color32::WHITE);
            }
        }
        if resp.hovered() {
            p.rect_filled(img, 8, Color32::from_black_alpha(30));
        }
        let name: String = if c.name.chars().count() > 24 { c.name.chars().take(22).chain("…".chars()).collect() } else { c.name.clone() };
        p.text(rect.min + vec2(2.0, side + 6.0), egui::Align2::LEFT_TOP, name, egui::FontId::proportional(house::BODY), theme::TEXT);
        let n = format!("{} {}", c.count, if c.count == 1 { "picture" } else { "pictures" });
        p.text(rect.min + vec2(2.0, side + 24.0), egui::Align2::LEFT_TOP, n, egui::FontId::proportional(house::NOTE), theme::TEXT_DIM);
        resp.clicked()
    }

    /// Under the bar: the name box, the delete question, and what to do with the selection.
    pub(super) fn pick_ui(&mut self, ui: &mut Ui, store: &Backend) {
        let ctx = ui.ctx().clone();
        let open = self.alb.open.clone().filter(|_| self.alb.tab == Tab::Albums);
        match self.alb.ask {
            Ask::New | Ask::Rename => {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(if self.alb.ask == Ask::New { "Album name" } else { "New name" }).size(house::BODY).color(theme::TEXT_DIM),
                    );
                    let r = ui.add(egui::TextEdit::singleline(&mut self.alb.text).desired_width(220.0));
                    let go = self.pill(ui, "OK", false) || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                    if go && !self.alb.text.trim().is_empty() {
                        let name = self.alb.text.trim().to_string();
                        if self.alb.ask == Ask::Rename {
                            if let Some(c) = &open {
                                self.rename(&ctx, store, &c.id, &name);
                            }
                        } else {
                            self.create(&ctx, store, &name, false);
                        }
                        self.alb.ask = Ask::None;
                    }
                    if self.pill(ui, "Cancel", false) {
                        self.alb.ask = Ask::None;
                    }
                });
            }
            Ask::Delete => {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Delete this album? The pictures stay where they are.").size(house::BODY).color(theme::TEXT));
                    if self.pill(ui, "Delete", false) {
                        if let Some(c) = &open {
                            self.delete(&ctx, store, &c.id);
                        }
                        self.alb.ask = Ask::None;
                    }
                    if self.pill(ui, "Cancel", false) {
                        self.alb.ask = Ask::None;
                    }
                });
            }
            Ask::None => {}
        }
    }

    fn create(&mut self, ctx: &egui::Context, store: &Backend, name: &str, all: bool) {
        let files = if all { self.pics.iter().map(|p| p.id.clone()).collect() } else { self.selection() };
        post(ctx, store, &self.inbox, "api/photos/albums", json!({"name": name, "files": files}));
        self.clear_selection();
    }

    fn rename(&mut self, ctx: &egui::Context, store: &Backend, id: &str, name: &str) {
        post(ctx, store, &self.inbox, &format!("api/photos/albums/{id}/rename"), json!({"name": name}));
        if let Some(c) = &mut self.alb.open {
            c.name = name.to_string();
        }
    }

    fn delete(&mut self, ctx: &egui::Context, store: &Backend, id: &str) {
        post(ctx, store, &self.inbox, &format!("api/photos/albums/{id}/delete"), json!({}));
        self.alb.open = None;
        self.clear_selection();
        self.pics.clear();
        self.viewer = None;
    }

    /// The test hook (TESTING.md): `home:photos:tab`, `:select`, `:album:new|open|back|add|remove|rename|delete`, `:place:open`.
    #[cfg(target_arch = "wasm32")]
    pub(in crate::home) fn alb_test(&mut self, ctx: &egui::Context, store: &Backend, id: &str, p: &Json) -> Result<(), String> {
        let s = |k: &str| p.get(k).and_then(Json::as_str).unwrap_or("").to_string();
        let find = |v: &[Card], k: &str| v.iter().find(|c| c.id == k || c.name == k).cloned();
        match id {
            "home:photos:tab" => {
                let t = match s("tab").as_str() {
                    "photos" => Tab::Photos,
                    "albums" => Tab::Albums,
                    "places" => Tab::Places,
                    "events" => Tab::Events,
                    "duplicates" => Tab::Duplicates,
                    _ => return Err("tab is photos, albums, places, events or duplicates".into()),
                };
                self.go(t);
            }
            "home:photos:select" => {
                self.clear_selection();
                if p.get("on").and_then(Json::as_bool).unwrap_or(true) {
                    if p.get("all").and_then(Json::as_bool) == Some(true) {
                        self.alb.selected = self.pics.iter().map(|x| x.id.clone()).collect();
                    }
                    for i in p.get("indexes").and_then(Json::as_array).into_iter().flatten().filter_map(Json::as_u64) {
                        let x = self.pics.get(i as usize).ok_or("no such picture")?;
                        self.alb.selected.insert(x.id.clone());
                    }
                }
            }
            "home:photos:dups:keep" => {
                let g = p.get("group").and_then(Json::as_u64).unwrap_or(0) as usize;
                let i = p.get("index").and_then(Json::as_u64).unwrap_or(0) as usize;
                if self.dups.groups.get(g).and_then(|x| x.pics.get(i)).is_none() {
                    return Err("no such duplicate".into());
                }
                self.keep_dup(g, i);
            }
            "home:photos:album:new" => {
                let n = s("name");
                self.create(ctx, store, &n, false);
            }
            "home:photos:album:open" => {
                let c = find(&self.alb.albums, &s("album")).ok_or("no such album")?;
                self.alb.tab = Tab::Albums;
                self.open_card(ctx, store, c);
            }
            "home:photos:place:open" => {
                self.alb.tab = Tab::Places;
                self.alb.boot_cards(ctx, store, &self.inbox);
                let c = find(&self.alb.places, &s("place")).ok_or("no such place (yet)")?;
                self.open_card(ctx, store, c);
            }
            "home:photos:event:open" => {
                self.alb.tab = Tab::Events;
                self.alb.boot_cards(ctx, store, &self.inbox);
                let c = find(&self.alb.events, &s("event")).ok_or("no such event (yet)")?;
                self.open_card(ctx, store, c);
            }
            "home:photos:event:save" => {
                let c = self.alb.open.clone().filter(|_| self.alb.tab == Tab::Events).ok_or("no event is open")?;
                self.create(ctx, store, &c.name, true);
            }
            "home:photos:album:back" => {
                self.alb.open = None;
                self.pics.clear();
            }
            "home:photos:album:add" => {
                let c = find(&self.alb.albums, &s("album")).ok_or("no such album")?;
                let ids = self.selection();
                self.add_to(ctx, store, &c, ids);
            }
            _ => {
                let c = self.alb.open.clone().filter(|_| self.alb.tab == Tab::Albums).ok_or("no album is open")?;
                match id {
                    "home:photos:album:remove" => {
                        let ids = self.selection();
                        self.act(ctx, store, super::pick::Act::Remove, ids, None);
                    }
                    "home:photos:album:rename" => self.rename(ctx, store, &c.id, &s("name")),
                    "home:photos:album:delete" => self.delete(ctx, store, &c.id),
                    _ => return Err(format!("unknown command `{id}`")),
                }
            }
        }
        Ok(())
    }
}
