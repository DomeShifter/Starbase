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

pub(super) mod albums;
mod dups;
mod justify;
mod locked;
mod map;
mod pick;
pub(super) mod shell;
mod timeline;
mod upload;

pub(super) use pick::{Act, Out};

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
/// The gap between pictures (the rows' target height: justify.rs).
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
    /// Starred by me (the server marks every listed picture, wherever it lives).
    pub starred: bool,
    pub size: u64,
    pub camera: Option<String>,
    /// Where it was taken, only when the server says this person may see it.
    pub gps: Option<(f64, f64)>,
    /// The town (or rounded position) of `gps`, named by the server.
    pub place: Option<String>,
    /// The media type ("image/jpeg", "video/mp4"...).
    pub media: String,
    /// A video's length, seconds.
    pub duration: Option<f64>,
    /// Camera RAW files stacked under this JPEG (same base name, same folder).
    pub stack: Vec<StackFile>,
}

/// A file stacked under a picture: its RAW twin.
#[derive(Clone, Debug, Default)]
pub(super) struct StackFile {
    pub id: String,
    pub name: String,
    pub v: Option<String>,
    pub size: u64,
}

impl Pic {
    /// This picture's id and its stacked files' ids (what Download and Delete act on).
    pub fn with_stack(&self) -> Vec<String> {
        std::iter::once(self.id.clone()).chain(self.stack.iter().map(|f| f.id.clone())).collect()
    }

    pub fn is_video(&self) -> bool {
        self.media.starts_with("video/")
    }
}

/// What to show for a failed request: the server's own words, except "no such file" (a picture
/// moved or deleted since the list was drawn), which read like a leaked internal message at the
/// top of Photos (To-do 3 row 43).
pub(crate) fn said(v: &Json, status: u16) -> String {
    said_in(v, status, "")
}

/// Like [`said`], with a prefix for the fallback ("Not saved: the server said 500.").
pub(crate) fn said_in(v: &Json, status: u16, what: &str) -> String {
    match v["error"].as_str() {
        Some(e) if e.trim().eq_ignore_ascii_case("no such file") => "That picture isn't there any more. It was moved or deleted.".to_string(),
        Some(e) => e.to_string(),
        None if what.is_empty() => format!("The server said {status}."),
        None => format!("{what}: the server said {status}."),
    }
}

#[cfg(test)]
mod said_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn no_such_file_is_never_shown_raw() {
        let t = said(&json!({"error": "no such file"}), 404);
        assert!(!t.contains("no such file") && t.contains("isn't there"), "{t}");
        assert_eq!(said_in(&json!({"error": "no such file"}), 404, "Not saved"), t);
    }

    #[test]
    fn other_messages_pass_through() {
        assert_eq!(said(&json!({"error": "You don't have access to that."}), 403), "You don't have access to that.");
        assert_eq!(said(&json!({}), 500), "The server said 500.");
        assert_eq!(said_in(&json!({}), 500, "No copy made"), "No copy made: the server said 500.");
    }
}

/// A video's length as the tile shows it: `0:07`, `12:30`, `1:02:03`.
pub(super) fn duration_label(secs: f64) -> String {
    let t = secs.max(0.0).round() as u64;
    let (h, m, s) = (t / 3600, t / 60 % 60, t % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// A video's tile: a play triangle in a dark disc in the middle, its length at the bottom right.
/// A stack's tile: a small dark chip with the number of files, bottom left.
fn stack_badge(painter: &egui::Painter, rect: Rect, n: usize) {
    let chip = Rect::from_min_size(rect.left_bottom() + vec2(6.0, -26.0), vec2(34.0, 20.0));
    painter.rect_filled(chip, 10, Color32::from_black_alpha(170));
    painter.text(chip.center(), egui::Align2::CENTER_CENTER, n.to_string(), egui::FontId::proportional(12.0), Color32::WHITE);
}

fn video_badge(painter: &egui::Painter, rect: Rect, duration: Option<f64>) {
    let c = rect.center();
    let r = (rect.width() * 0.16).clamp(14.0, 30.0);
    painter.circle_filled(c, r, Color32::from_black_alpha(150));
    painter.add(egui::Shape::convex_polygon(
        vec![c + vec2(-r * 0.3, -r * 0.45), c + vec2(-r * 0.3, r * 0.45), c + vec2(r * 0.5, 0.0)],
        Color32::WHITE,
        egui::Stroke::NONE,
    ));
    if let Some(d) = duration {
        let font = egui::FontId::proportional(house::NOTE);
        let g = painter.layout_no_wrap(duration_label(d), font, Color32::WHITE);
        let pos = rect.right_bottom() - vec2(6.0, 6.0) - g.size();
        painter.rect_filled(Rect::from_min_size(pos, g.size()).expand2(vec2(4.0, 2.0)), 4, Color32::from_black_alpha(170));
        painter.galley(pos, g, Color32::WHITE);
    }
}

/// Whether a Favourite click turns stars on: like Google Photos, any not yet a favourite → all
/// become one, else all stop. `photos` is Photos' own listing, which holds pictures in subfolders
/// and albums; `drive` (the open folder's files) is only a fallback for a picture not listed there.
pub(super) fn stars_on(ids: &[String], photos: &[Pic], drive: impl Fn(&str) -> Option<bool>) -> bool {
    !ids.iter().all(|id| photos.iter().find(|p| &p.id == id).map(|p| p.starred).or_else(|| drive(id)).unwrap_or(false))
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
    /// Every picture with a position, for the Places map (map.rs).
    Map(Json),
    /// The Duplicates list for a folder (dups.rs).
    Dups(String, Json),
    Opened(albums::Tab, String, Json),
    /// An album changed: ask again.
    Changed,
    /// Pictures went into an album (pick.rs): the album, how many were new, how many were asked.
    Added(albums::Card, u64, u64, Vec<String>),
    /// The "Add photos" picker's list (pick.rs).
    Picker(Json),
    /// Locked (locked.rs): its state, a password accepted or refused, pictures moved in (true) or out.
    LockedState(Json),
    Unlocked,
    Wrong(String),
    Moved(Vec<String>, bool),
    /// A picture was turned (id, left, the new thumbnail version) or couldn't be.
    Rotated(String, bool, Result<Option<String>, String>),
    Nothing,
    /// A file picked for upload (name, bytes), and the answer for upload number n (true: a duplicate).
    Picked(String, Vec<u8>),
    Uploaded(usize, Result<bool, String>),
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

/// The setting that remembers the Locked folder note was dismissed ("1").
pub const LOCKED_HINT_KEY: &str = "starsheets_home_photos_hint_locked";
/// The one-time note shown above the pictures in Photo Mode.
pub const LOCKED_HINT: &str = "Hide private pictures: right-click a picture and choose Move to Locked folder. Find them under Locked on the left.";

pub(super) struct PhotoMode {
    /// The switch: show folders as photos.
    pub on: bool,
    booted: bool,
    /// The one-time "how to hide pictures" note: Got it was pressed (a setting, kept).
    hint_dismissed: bool,
    /// The note was drawn this frame (for the test hook).
    hint_shown: bool,
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
    /// The real width / height of pictures whose thumbnail has loaded and differs from the
    /// listing's (turned by EXIF or by an edit): the grid lays out with it.
    ratios: HashMap<String, f32>,
    /// Rotations asked for and not yet answered, and the ones that worked (one Undo for all).
    turning: (usize, Vec<super::undo::Step>),
    failed: HashSet<Key>,
    pub frame: u64,
    pub viewer: Option<Viewer>,
    /// The photo editor, over the viewer (photo_edit.rs).
    pub editor: Option<super::photo_edit::Editor>,
    /// Albums and Places beside the grid (albums.rs).
    pub alb: albums::Albums,
    /// Places as a world map (map.rs).
    pub(super) map: map::PlacesMap,
    /// The Duplicates tab (dups.rs).
    pub dups: dups::Dups,
    /// Selecting, the right-click menu, Add to album (pick.rs).
    pub pick: pick::Pick,
    /// The Locked tab (locked.rs).
    pub lk: locked::Locked,
    /// The Timeline tab (timeline.rs).
    pub tl: timeline::Timeline,
    /// Upload from this device (upload.rs).
    pub up: upload::Uploads,
    /// Photo Mode's room: dark look, set of pictures, type filter, search (shell.rs).
    pub shell: shell::Shell,
    /// What the Drive around Photos is asked to do (stars, the bin, Details).
    pub out: Vec<Out>,
}

impl Default for PhotoMode {
    fn default() -> PhotoMode {
        PhotoMode {
            on: false,
            booted: false,
            hint_dismissed: false,
            hint_shown: false,
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
            up: Default::default(),
            shell: Default::default(),
            inbox: Arc::default(),
            tex: Textures { budget: BUDGET, ..Default::default() },
            asking: HashSet::new(),
            retry_at: HashMap::new(),
            ratios: HashMap::new(),
            turning: (0, Vec::new()),
            failed: HashSet::new(),
            frame: 0,
            viewer: None,
            editor: None,
            alb: albums::Albums::default(),
            map: map::PlacesMap::default(),
            dups: dups::Dups::default(),
            pick: pick::Pick::default(),
            lk: locked::Locked::default(),
            tl: timeline::Timeline::default(),
            out: Vec::new(),
        }
    }
}

impl PhotoMode {
    pub fn boot(&mut self, store: &Backend) {
        if !self.booted {
            self.booted = true;
            self.on = store.setting("starsheets_home_photos") == Some("1".into());
            self.newest_first = store.setting("starsheets_home_photos_order") != Some("old".into());
            self.hint_dismissed = store.setting(LOCKED_HINT_KEY).as_deref() == Some("1");
        }
    }

    /// "Got it" on the Locked folder note: never shown again for this person.
    pub fn dismiss_locked_hint(&mut self, store: &Backend) {
        self.hint_dismissed = true;
        store.set_setting(LOCKED_HINT_KEY, "1");
    }

    /// Drive's "Locked" entry: Photo Mode on, at the Locked tab (which asks for the password).
    pub(in crate::home) fn open_locked(&mut self, store: &Backend) {
        self.boot(store);
        self.set_on(store, true);
        self.go(albums::Tab::Locked);
    }

    /// The one-time note above the grid on how to hide pictures and where to find them.
    fn locked_hint_ui(&mut self, ui: &mut Ui, store: &Backend) {
        if self.hint_dismissed {
            return;
        }
        self.hint_shown = true;
        ui.add_space(house::S);
        let card = house::card_frame().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if let Some(img) = crate::icons::icon("lock") {
                    ui.add(img.fit_to_exact_size(vec2(18.0, 18.0)));
                }
                ui.label(RichText::new(LOCKED_HINT).size(house::BODY).color(theme::text()));
                let ok = ui.add(house::link("Got it"));
                crate::house::target(ui.ctx(), "photos:hint-locked:ok", ok.rect);
                if ok.clicked() {
                    self.dismiss_locked_hint(store);
                }
            });
        });
        crate::house::target(ui.ctx(), "photos:hint-locked", card.response.rect);
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
        let url = if self.shell.scope == shell::Scope::All {
            format!("api/photos?all=1&sort=taken{order}")
        } else {
            format!("api/photos?folder={}&sort=taken{order}{}", enc(folder), if self.sub { "&sub=1" } else { "" })
        };
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
                        Got::Error(crate::home::photos::said(&v, s))
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
                        Got::Error(crate::home::photos::said(&v, s))
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
                starred: p["starred"].as_bool().unwrap_or(false),
                size: p["size"].as_u64().unwrap_or(0),
                camera: p["camera"].as_str().map(str::to_string),
                gps: p["gps"]["lat"].as_f64().zip(p["gps"]["lon"].as_f64()),
                place: p["gps"]["place"].as_str().map(str::to_string),
                media: s(p, "media"),
                duration: p["duration"].as_f64(),
                stack: p["stack"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|f| StackFile {
                        id: s(f, "id"),
                        name: s(f, "name"),
                        v: f["v"].as_str().map(str::to_string),
                        size: f["size"].as_u64().unwrap_or(0),
                    })
                    .collect(),
            })
            .collect()
    }

    /// Shows the new star state at once (the listing isn't fetched again for it).
    pub(super) fn set_starred(&mut self, ids: &[String], on: bool) {
        for p in self.pics.iter_mut().filter(|p| ids.contains(&p.id)) {
            p.starred = on;
        }
    }

    fn set_pics(&mut self, pics: Vec<Pic>) {
        // A picture read since: its thumbnail may be asked for again (it was "not yet").
        for p in &pics {
            if p.v.is_some() {
                self.retry_at.remove(&(p.id.clone(), SMALL));
            }
        }
        self.shell.full = pics;
        self.refilter();
    }

    /// Each frame: answers, and the listing again while pictures are still being read.
    pub fn logic(&mut self, ctx: &egui::Context, store: &Backend, folder: Option<String>) {
        self.frame += 1;
        self.upload_pump(ctx, store, folder.as_deref(), ctx.input(|i| i.time));
        let now = ctx.input(|i| i.time);
        let got: Vec<Got> = std::mem::take(&mut *self.inbox.lock().unwrap_or_else(|e| e.into_inner()));
        for g in got {
            match g {
                Got::Cards(t, v) => self.alb.take_cards(t, &v),
                Got::Map(v) => self.map.take(&v),
                Got::Opened(t, key, v) => self.take_opened(t, &key, &v),
                Got::Dups(f, v) => self.dups.take(&f, &v),
                Got::Changed => self.alb.changed(ctx, store, &self.inbox),
                Got::Added(c, n, asked, ids) => self.added(ctx, store, c, n, asked, ids),
                Got::Picker(v) => self.take_picker(&v),
                Got::LockedState(v) => self.take_locked(ctx, &v),
                Got::Unlocked => self.unlocked(ctx, store),
                Got::Wrong(e) => {
                    self.lk.busy = false;
                    self.lk.error = Some(e);
                }
                Got::Moved(ids, into) => self.moved(ctx, store, &ids, into),
                Got::Rotated(id, left, r) => self.rotated(ctx, id, left, r),
                Got::Nothing => {}
                Got::Picked(name, bytes) => {
                    let f = folder.clone().or_else(|| self.folder.clone()).unwrap_or_default();
                    self.upload_add(&f, name, bytes);
                }
                Got::Uploaded(i, r) => self.upload_answer(i, r, now),
                Got::Listed(f, sub, v) => {
                    if Some(&f) == self.folder.as_ref() && sub == self.sub && matches!(self.alb.tab, albums::Tab::Photos | albums::Tab::Timeline) {
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
            && matches!(self.alb.tab, albums::Tab::Photos | albums::Tab::Timeline)
            && (self.waiting > 0 || self.pics.iter().any(|p| p.state == "pending"))
            && now - self.listed_at > RELIST
        {
            self.list(ctx, store, f);
        }
        if self.waiting > 0 {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(RELIST));
        }
        self.locked_logic(ctx, store);
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
        let url = if let Some(t) = id.strip_prefix("tile/") {
            format!("api/photos/tiles/{t}.png")
        } else if size == ORIGINAL {
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

    /// Turns pictures a quarter (not videos or ones that can't be shown), as a saved edit.
    pub(super) fn rotate(&mut self, ctx: &egui::Context, store: &Backend, ids: Vec<String>, left: bool) {
        let ok = |p: &Pic| !p.is_video() && !matches!(p.state.as_str(), "failed" | "unsupported");
        let ids: Vec<String> = ids.into_iter().filter(|id| self.pics.iter().any(|p| &p.id == id && ok(p))).collect();
        if ids.is_empty() || self.turning.0 > 0 {
            return;
        }
        self.turning = (ids.len(), Vec::new());
        for id in ids {
            let (inbox, ctx2, id2) = (self.inbox.clone(), ctx.clone(), id.clone());
            store.call(
                ctx,
                Method::Post,
                &format!("api/photos/rotate/{}", enc(&id)),
                Some(super::undo::rotate_body(left)),
                Box::new(move |a| {
                    let r = match a {
                        Ok((200, b)) => Ok(serde_json::from_slice::<Json>(&b).ok().and_then(|v| v["thumb_v"].as_str().map(str::to_string))),
                        Ok((s, b)) => {
                            let v: Json = serde_json::from_slice(&b).unwrap_or_default();
                            Err(crate::home::photos::said(&v, s))
                        }
                        Err(e) => Err(e),
                    };
                    Self::push(&inbox, &ctx2, Got::Rotated(id2, left, r));
                }),
            );
        }
    }

    /// One picture's turn answered: its thumbnail is asked for again, and when all are, Undo is offered.
    fn rotated(&mut self, ctx: &egui::Context, id: String, left: bool, r: Result<Option<String>, String>) {
        self.turning.0 = self.turning.0.saturating_sub(1);
        match r {
            Ok(v) => {
                // The grid already lays it out turned; the new thumbnail confirms it.
                let was = self.pics.iter().find(|p| p.id == id).map_or(4.0 / 3.0, |p| self.ratio_of(p));
                self.edited(&id, v);
                self.ratios.insert(id.clone(), 1.0 / was);
                self.turning.1.push(super::undo::Step::Rotate { id, left });
            }
            Err(e) => self.error = Some(format!("Couldn't turn the picture: {e}")),
        }
        if self.turning.0 == 0 && !self.turning.1.is_empty() {
            let steps = std::mem::take(&mut self.turning.1);
            let label = format!("Rotated {}", if left { "left" } else { "right" });
            self.say_undo(ctx, label.clone());
            self.out.push(Out::Did(label, steps));
        }
    }

    /// Picture `id`'s edits were saved: its thumbnails are asked for again at version `v`
    /// (the server's thumbnails show the edited picture).
    pub fn edited(&mut self, id: &str, v: Option<String>) {
        self.tex.forget(id);
        self.ratios.remove(id);
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
        self.best_id(&id, prefer)
    }

    /// The same for a file by its id (a stacked RAW).
    pub fn best_id(&mut self, id: &str, prefer: u32) -> Option<(egui::TextureHandle, u32)> {
        let id = id.to_string();
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
        self.hint_shown = false;
        self.alb.boot_cards(&ctx, store, &self.inbox);
        // On a phone the nav is a wrapped row above the pictures, not a column beside them.
        if ui.available_width() < 560.0 {
            ui.horizontal_wrapped(|ui| self.nav_ui(ui, store));
            ui.separator();
            self.tab_ui(ui, store, folder);
            return;
        }
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(130.0);
                self.nav_ui(ui, store);
            });
            ui.separator();
            ui.vertical(|ui| self.tab_ui(ui, store, folder));
        });
    }

    fn tab_ui(&mut self, ui: &mut Ui, store: &Backend, folder: &str) {
        match self.alb.tab {
            albums::Tab::Photos | albums::Tab::Timeline => self.folder_ui(ui, store, folder),
            albums::Tab::Duplicates => self.dups_ui(ui, store, folder),
            albums::Tab::Locked => self.locked_ui(ui, store),
            _ => self.cards_ui(ui, store),
        }
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
                ui.label(RichText::new(if self.loading { "Loading…" } else { "" }).size(house::BODY).color(theme::text_dim()));
                return;
            }
            Some(false) => {
                self.ask_ui(ui, store, folder);
                return;
            }
            Some(true) => {}
        }
        if self.alb.selected.is_empty() {
            self.bar_ui(ui, store, folder);
            self.locked_hint_ui(ui, store);
        } else {
            self.sel_bar_ui(ui, store);
        }
        self.pick_ui(ui, store);
        self.upload_ui(ui);
        ui.add_space(house::S);
        if self.pics.is_empty() {
            ui.add_space(house::XL);
            let text = if self.loading {
                "Loading…"
            } else if self.shell.filtering() && !self.shell.full.is_empty() {
                "No pictures match the search or the type."
            } else if self.shell.scope == shell::Scope::All {
                "No pictures yet. Turn on Photos in a folder to see its pictures here."
            } else {
                "No pictures in this folder."
            };
            ui.label(RichText::new(text).size(house::BODY).color(theme::text_dim()));
            return;
        }
        if self.alb.tab == albums::Tab::Timeline {
            self.timeline_ui(ui, store);
        } else {
            self.grid(ui, store);
        }
    }

    /// The ratio (width / height) picture `i` is laid out with.
    fn ratio_of(&self, p: &Pic) -> f32 {
        self.ratios.get(&p.id).copied().unwrap_or_else(|| justify::ratio(p.width, p.height))
    }

    /// The scrolling grid of `pics`: justified rows, every picture in its own aspect ratio.
    pub(super) fn grid(&mut self, ui: &mut Ui, store: &Backend) {
        let ctx = ui.ctx().clone();
        let width = (ui.available_width() - house::SCROLL_ROOM).max(80.0);
        let ratios: Vec<f32> = self.pics.iter().map(|p| self.ratio_of(p)).collect();
        let rows = justify::rows(&ratios, width, justify::target_for(width), GAP);
        let (tops, total) = justify::offsets(&rows, GAP);
        let mut hit = None;
        let mut seen = 0..0;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_viewport(ui, |ui, view| {
            let (_, area) = ui.allocate_space(vec2(width, total));
            // One screen above and below what is shown.
            let (lo, hi) = (view.min.y - view.height(), view.max.y + view.height());
            let first = tops.partition_point(|t| *t < lo).saturating_sub(1);
            let last = tops.partition_point(|t| *t < hi).min(rows.len());
            seen = first..last;
            for (row, top) in rows[first..last].iter().zip(&tops[first..last]) {
                let mut x = 0.0;
                for (i, r) in ratios.iter().enumerate().take(row.end).skip(row.start) {
                    let w = r * row.height;
                    let rect = Rect::from_min_size(area.min + vec2(x, *top), vec2(w, row.height));
                    x += w + GAP;
                    // Only what is on screen is asked for first; the rest below.
                    if rect.max.y < area.min.y + view.min.y || rect.min.y > area.min.y + view.max.y {
                        continue;
                    }
                    if let Some(h) = self.tile_ui(ui, &ctx, store, i, rect) {
                        hit = Some((i, h));
                    }
                }
            }
        });
        // A screen ahead, after everything on screen was asked for.
        let n = self.pics.len();
        let ahead = rows.get(seen.end..).unwrap_or(&[]).iter().take(seen.len().max(1));
        for i in ahead.flat_map(|r| r.start..r.end.min(n)) {
            self.want(&ctx, store, i, SMALL);
        }
        match hit {
            Some((i, Hit::Click(on_box))) => {
                let m = ctx.input(|i| i.modifiers);
                self.clicked(i, on_box, m.shift, m.command);
            }
            // While selecting, a double-click still opens the picture.
            Some((i, Hit::Double)) => self.viewer = Some(Viewer::new(i, self.pics.len())),
            Some((i, Hit::Menu(act, ids))) => self.act(&ctx, store, act, ids, Some(i)),
            None => {}
        }
        // Esc clears the selection, Ctrl/⌘-A selects every picture listed (when no viewer, dialog
        // or text box has the keys).
        let free = self.viewer.is_none() && self.pick.dialog.is_none() && self.pick.picker.is_none() && ctx.memory(|m| m.focused().is_none());
        if free && !self.alb.selected.is_empty() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.clear_selection();
        }
        if free && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::A)) {
            self.select_all();
        }
        // Delete moves the selection to the bin (Shift-Delete: out of the open album only).
        if free && !self.alb.selected.is_empty() {
            let (shift, plain) = ctx
                .input_mut(|i| (i.consume_key(egui::Modifiers::SHIFT, egui::Key::Delete), i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)));
            let ids = self.selection();
            if shift && self.open_album().is_some() {
                self.act(&ctx, store, Act::Remove, ids, None);
            } else if plain || shift {
                self.act(&ctx, store, Act::Delete, ids, None);
            }
        }
        if free && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
            self.out.push(Out::Undo);
        }
    }

    /// One square of the grid: a click (on its checkbox or not), or a right-click menu command.
    fn tile_ui(&mut self, ui: &mut Ui, ctx: &egui::Context, store: &Backend, i: usize, rect: Rect) -> Option<Hit> {
        let resp = ui.interact(rect, ui.auto_id_with(("photo_tile", i)), Sense::click());
        self.want(ctx, store, i, SMALL);
        let p = &self.pics[i];
        let (name, date, state) = (p.name.clone(), date_label(p.taken.as_deref(), p.modified), p.state.clone());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0, Color32::from_rgb(0xE8, 0xEA, 0xED));
        if let Some((t, _)) = self.best(i, SMALL) {
            let [w, h] = t.size();
            painter.image(t.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            // The picture's real shape (turned by EXIF or an edit) replaces the listing's.
            let real = justify::ratio(w as u32, h as u32);
            let id = self.pics[i].id.clone();
            let listed = justify::ratio(self.pics[i].width, self.pics[i].height);
            if self.ratios.get(&id).map_or((real / listed - 1.0).abs() > 0.02, |r| (real / r - 1.0).abs() > 0.02) {
                self.ratios.insert(id, real);
            }
        } else if self.pics[i].is_video() {
            painter.rect_filled(rect, 0, Color32::from_rgb(0x3C, 0x40, 0x43));
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
                theme::text_dim(),
            );
        }
        if self.pics[i].is_video() {
            video_badge(&painter, rect, self.pics[i].duration);
        }
        if !self.pics[i].stack.is_empty() {
            stack_badge(&painter, rect, 1 + self.pics[i].stack.len());
        }
        if resp.hovered() {
            painter.rect_filled(rect, 0, Color32::from_black_alpha(40));
        }
        let on = self.alb.selected.contains(&self.pics[i].id);
        pick::check(&painter, rect, on, resp.hovered() || !self.alb.selected.is_empty());
        // Right-click: the picture joins (or becomes) the selection, and the menu acts on it all.
        if resp.secondary_clicked() {
            let ids: Vec<String> = self.pics.iter().map(|p| p.id.clone()).collect();
            pick::targets(&mut self.alb.selected, &mut self.pick.anchor, &ids, i);
        }
        let in_album = self.open_album().is_some();
        let mut chosen = None;
        resp.context_menu(|ui| {
            house::menu(ui, 240.0);
            for (act, icon, label) in pick::menu_rows(in_album, self.locked_tab()) {
                if house::menu_row(ui, Some(icon), label, "", true) {
                    chosen = Some(act);
                    ui.close();
                }
            }
        });
        if let Some(act) = chosen {
            return Some(Hit::Menu(act, self.selection()));
        }
        let resp = resp.on_hover_text(format!("{name}\n{date}"));
        if resp.double_clicked() && !self.alb.selected.is_empty() {
            return Some(Hit::Double);
        }
        if resp.clicked() {
            let on_box = resp.interact_pointer_pos().is_some_and(|p| pick::box_rect(rect).contains(p));
            return Some(Hit::Click(on_box));
        }
        None
    }

    /// Above the grid: how many, subfolders, order, slideshow, turning off.
    fn bar_ui(&mut self, ui: &mut Ui, store: &Backend, folder: &str) {
        let ctx = ui.ctx().clone();
        ui.horizontal_wrapped(|ui| {
            let n = self.pics.len();
            let mut text = format!("{n} {}", if n == 1 { "picture" } else { "pictures" });
            if self.waiting > 0 {
                text.push_str(&format!(" · preparing {}", self.waiting));
            }
            ui.label(RichText::new(text).size(house::BODY).color(theme::text_dim()));
            ui.add_space(house::M);
            let pill = |ui: &mut Ui, label: &str, on: bool| -> egui::Response {
                ui.add(
                    egui::Button::new(RichText::new(label).size(house::BODY).color(theme::text()))
                        .fill(if on { theme::share() } else { theme::card() })
                        .stroke(egui::Stroke::new(1.0, theme::edge()))
                        .corner_radius(16)
                        .min_size(vec2(0.0, 32.0)),
                )
            };
            if pill(ui, if self.newest_first { "Newest first" } else { "Oldest first" }, false).clicked() {
                self.newest_first = !self.newest_first;
                store.set_setting("starsheets_home_photos_order", if self.newest_first { "new" } else { "old" });
                self.list(&ctx, store, folder);
            }
            let up = pill(ui, "Upload", false);
            self.up.btn = Some([up.rect.center().x, up.rect.center().y]);
            if up.clicked() {
                self.upload_pick(&ctx, store);
            }
            if !self.pics.is_empty() && pill(ui, "Slideshow", false).clicked() {
                let mut v = Viewer::new(0, self.pics.len());
                v.play(ui.ctx().input(|i| i.time));
                self.viewer = Some(v);
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
            ui.label(RichText::new("Show this folder as photos?").size(house::TITLE).color(theme::text()));
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
            "scope": self.shell.scope.label(),
            "types": self.shell.types.iter().enumerate().filter(|(_, t)| **t).map(|(i, _)| shell::KINDS[i]).collect::<Vec<_>>(),
            "dark": crate::theme::photo_dark(),
            "query": self.shell.query,
            "total": self.shell.full.len(),
            "newestFirst": self.newest_first,
            "loading": self.loading,
            "waiting": self.waiting,
            "error": self.error,
            "count": self.pics.len(),
            "timeline": self.tl.test_state(),
            "albums": self.alb.test_state(),
            "map": self.map.test_state(),
            "dups": self.dups.test_state(),
            "pick": self.pick.test_state(),
            "locked": self.locked_state(),
            "lockedHint": { "shown": self.hint_shown, "dismissed": self.hint_dismissed },
            "upload": self.up.test_state(),
            "textures": { "count": self.tex.len(), "bytes": self.tex.total, "budget": self.tex.budget, "asking": self.asking.len() },
            "items": self.pics.iter().take(60).map(|p| json!({
                "id": p.id, "name": p.name, "taken": p.taken, "state": p.state, "v": p.v, "media": p.media, "duration": p.duration, "stack": p.stack.iter().map(|f| &f.name).collect::<Vec<_>>(),
                "thumb": has(&p.id, SMALL), "large": has(&p.id, LARGE), "original": has(&p.id, ORIGINAL),
            })).collect::<Vec<_>>(),
            "photo_edit": self.editor.as_ref().map_or_else(|| json!({ "open": false }), |e| e.test_state()),
            "viewer": self.viewer.as_ref().map(|v| {
                let p = self.pics.get(v.index());
                json!({
                    "index": v.index(), "pos": v.pos, "id": p.map(|p| &p.id), "name": p.map(|p| &p.name),
                    "info": v.info, "menu": v.menu, "toast": crate::house::toast_text(), "info_rows": p.map(|p| super::viewer::info_lines(p, &self.alb.albums_of(&p.id)).iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>()), "zoom": v.zoom, "playing": v.playing, "shuffle": v.shuffle, "every": v.every, "showing": v.showing, "raw": v.raw,
                })
            }),
        })
    }
}

/// What happened on a grid square.
enum Hit {
    /// A left click; true when on its checkbox.
    Click(bool),
    Double,
    /// A right-click menu command, on these pictures.
    Menu(pick::Act, Vec<String>),
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

    /// Remembers settings like a browser's local storage; nothing else.
    #[derive(Default)]
    struct Settings(std::cell::RefCell<HashMap<String, String>>);

    impl crate::store::Store for Settings {
        fn call(&self, _: &egui::Context, _: crate::store::Method, _: &str, _: Option<Vec<u8>>, _: crate::store::Done) {}
        fn open(&self, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn new_file(&self, _: &str, _: bool) -> Result<(), String> {
            Ok(())
        }
        fn pick_files(&self, _: Box<dyn Fn(String, Vec<u8>)>) {}
        fn download(&self, _: &str, _: &[u8]) -> Result<(), String> {
            Ok(())
        }
        fn setting(&self, key: &str) -> Option<String> {
            self.0.borrow().get(key).cloned()
        }
        fn set_setting(&self, key: &str, value: &str) {
            self.0.borrow_mut().insert(key.into(), value.into());
        }
        fn has_photos(&self) -> bool {
            true
        }
    }

    #[test]
    fn locked_hint_shows_until_got_it_then_never_again() {
        let store = Backend(std::rc::Rc::new(Settings::default()));
        let mut p = PhotoMode::default();
        p.boot(&store);
        assert!(!p.hint_dismissed, "new person: the note shows");
        p.dismiss_locked_hint(&store);
        assert!(p.hint_dismissed);
        assert_eq!(store.setting(LOCKED_HINT_KEY).as_deref(), Some("1"), "kept as a setting");
        let mut again = PhotoMode::default();
        again.boot(&store);
        assert!(again.hint_dismissed, "next visit: not shown again");
    }

    #[test]
    fn drive_locked_entry_turns_photo_mode_on_at_locked() {
        let store = Backend(std::rc::Rc::new(Settings::default()));
        let mut p = PhotoMode::default();
        assert!(!p.on);
        p.open_locked(&store);
        assert!(p.on, "Photo Mode on");
        assert_eq!(store.setting("starsheets_home_photos").as_deref(), Some("1"), "and remembered");
        assert!(p.alb.tab == albums::Tab::Locked, "at the Locked tab (asks for the password)");
    }

    #[test]
    fn video_lengths_read_like_a_player() {
        assert_eq!([duration_label(7.4), duration_label(750.0), duration_label(3723.0)], ["0:07", "12:30", "1:02:03"]);
    }

    #[test]
    fn favourite_reads_the_star_of_pictures_in_subfolders() {
        let pic = |id: &str, starred| Pic { id: id.into(), starred, ..Pic::default() };
        // Photos lists a starred picture of a subfolder; the Drive's listing (open folder) lacks it.
        let pics = [pic("top", false), pic("sub", true), pic("top2", true)];
        let drive = |id: &str| (id == "top2" || id == "top").then_some(id == "top2");
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        for (sel, on, why) in [
            (ids(&["sub"]), false, "a starred picture in a subfolder is un-starred, not starred again"),
            (ids(&["sub", "top2"]), false, "all starred: all stop"),
            (ids(&["sub", "top"]), true, "one not yet a favourite: all become one"),
            (ids(&["top"]), true, "not starred"),
            (ids(&["gone"]), true, "unknown counts as not starred"),
        ] {
            assert_eq!(stars_on(&sel, &pics, drive), on, "{why}");
        }
        assert!(!stars_on(&ids(&["d"]), &[], |_| Some(true)), "the Drive answers for pictures Photos hasn't listed");
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
