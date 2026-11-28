//! Choosing pictures the Google Photos way: the round checkbox on a square, Shift and Ctrl
//! clicks, Esc; the bar that replaces the top bar while pictures are selected; the right-click
//! menu; the "Add to album…" dialog (with New album); "Add photos" inside an album; and the
//! note at the bottom ("Added 3 to Summer", Open album). The albums live on the server, per
//! person (`crates/server/src/photos/albums.rs`).

use std::collections::HashSet;

use egui::{Color32, Rect, RichText, Sense, Ui, vec2};
use serde_json::{Value as Json, json};

use super::albums::{Card, Tab, get, post};
use super::{Got, PhotoMode, Pic, SMALL};
use crate::house;
use crate::store::{Backend, Method, enc};
use crate::theme;

/// How long the note at the bottom stays (s).
const NOTE_SECS: f64 = 6.0;
/// A square in the "Add photos" picker.
const PICK_TILE: f32 = 104.0;

/// What a left click on a grid square came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Click {
    /// Nothing is selected and it was a plain click: open the viewer.
    Open,
    /// The selection changed.
    Picked,
}

/// The selection logic, without any UI: `ids` in grid order, square `i` clicked, `on_box` when
/// on its round checkbox. Like Google Photos: Shift adds the range from the last picked square;
/// Ctrl (⌘), the checkbox, or any click while something is selected toggles; else it opens.
pub(super) fn click(
    sel: &mut HashSet<String>,
    anchor: &mut Option<String>,
    ids: &[String],
    i: usize,
    on_box: bool,
    shift: bool,
    ctrl: bool,
) -> Click {
    let Some(id) = ids.get(i) else { return Click::Picked };
    if shift && let Some(a) = anchor.as_ref().and_then(|a| ids.iter().position(|x| x == a)) {
        let (lo, hi) = if a <= i { (a, i) } else { (i, a) };
        sel.extend(ids[lo..=hi].iter().cloned());
        return Click::Picked;
    }
    if !(shift || ctrl || on_box || !sel.is_empty()) {
        return Click::Open;
    }
    if !sel.remove(id) {
        sel.insert(id.clone());
    }
    *anchor = Some(id.clone());
    Click::Picked
}

/// The pictures a right-click on square `i` acts on: the whole selection (in grid order) when
/// the square is in it, else just that square (which becomes the selection, like Google Photos).
pub(super) fn targets(sel: &mut HashSet<String>, anchor: &mut Option<String>, ids: &[String], i: usize) -> Vec<String> {
    let Some(id) = ids.get(i) else { return Vec::new() };
    if !sel.contains(id) {
        sel.clear();
        sel.insert(id.clone());
        *anchor = Some(id.clone());
    }
    ids.iter().filter(|x| sel.contains(*x)).cloned().collect()
}

/// A command on the selected pictures (the right-click menu, the selection bar, the viewer's ⋮).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::home) enum Act {
    Open,
    AddTo,
    Favourite,
    Download,
    Info,
    Delete,
    /// Into the Locked folder, and back out of it (locked.rs).
    Lock,
    Unlock,
    /// Out of the open album only; the picture stays.
    Remove,
    /// Every picture listed (the bar's Select all, Ctrl/⌘-A).
    SelectAll,
    /// A quarter turn, saved as an edit (R and Shift-R in the viewer).
    RotateLeft,
    RotateRight,
}

/// The right-click menu's rows: (command, icon, label). "Remove from album" only in an album.
pub(super) fn menu_rows(in_album: bool, in_locked: bool) -> Vec<(Act, &'static str, &'static str)> {
    // Inside Locked only what the Locked folder allows (no bin, no albums, no downloads).
    if in_locked {
        return vec![(Act::Open, "open_in_new", "Open"), (Act::Unlock, "lock", "Move out of Locked folder")];
    }
    let mut v = vec![
        (Act::Open, "open_in_new", "Open"),
        (Act::AddTo, "photo_library", "Add to album…"),
        (Act::Favourite, "star", "Favourite"),
        (Act::Download, "download", "Download"),
        (Act::RotateLeft, "rotate_left", "Rotate left"),
        (Act::RotateRight, "rotate_right", "Rotate right"),
        (Act::Info, "info", "Info"),
    ];
    if in_album {
        v.push((Act::Remove, "remove", "Remove from album"));
    }
    if crate::home::LOCKED_PHOTOS_VISIBLE {
        v.push((Act::Lock, "lock", "Move to Locked folder…"));
    }
    v.push((Act::Delete, "delete", "Move to bin"));
    v
}

/// The selection bar: (command, icon, label, shown on the bar). The first four stay on the bar;
/// the rest sit in its More menu. "Remove from album" only in an album.
pub(super) fn sel_rows(in_album: bool) -> Vec<(Act, &'static str, &'static str, bool)> {
    let mut v = vec![
        (Act::AddTo, "photo_library", "Add to album", true),
        (Act::Favourite, "star", "Favourite", true),
        (Act::Download, "download", "Download", true),
        (Act::Delete, "delete", "Move to bin", true),
    ];
    if in_album {
        v.push((Act::Remove, "remove", "Remove from album", false));
    }
    v.push((Act::Info, "info", "Info", false));
    v
}

/// What the Drive around Photos does for it (it knows stars, the bin and Details).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::home) enum Out {
    Star(Vec<String>),
    Delete(Vec<String>),
    Info(String),
    /// A Photos action that is done, for Ctrl+Z (label, steps).
    Did(String, Vec<crate::home::undo::Step>),
    /// The note's Undo button (or Ctrl+Z in the viewer).
    Undo,
}

/// "Add to album…": the pictures, and the New album name while typing one.
#[derive(Clone, Debug, Default)]
pub(in crate::home) struct Dialog {
    pub ids: Vec<String>,
    pub naming: bool,
    pub text: String,
    /// The search box's text, and the highlighted row (↑/↓, Enter).
    pub search: String,
    pub cursor: usize,
    focused: bool,
}

/// How many albums the picker lists under "Recent".
const RECENT: usize = 5;
/// The setting that remembers them (most recent first, ids joined by commas).
const RECENT_KEY: &str = "starsheets_home_photos_recent_albums";

/// The picker's list: albums whose name holds `search` (any case); with no search, the
/// recently used ones first (up to RECENT, most recent first), then the rest A–Z.
/// Returns (album, under "Recent").
pub(super) fn picker_rows(albums: &[Card], recent: &[String], search: &str) -> Vec<(Card, bool)> {
    let q = search.trim().to_lowercase();
    let mut rest: Vec<Card> = albums.iter().filter(|c| q.is_empty() || c.name.to_lowercase().contains(&q)).cloned().collect();
    rest.sort_by_key(|c| c.name.to_lowercase());
    if !q.is_empty() {
        return rest.into_iter().map(|c| (c, false)).collect();
    }
    let mut out: Vec<(Card, bool)> =
        recent.iter().filter_map(|id| albums.iter().find(|c| &c.id == id)).take(RECENT).map(|c| (c.clone(), true)).collect();
    rest.retain(|c| !out.iter().any(|(r, _)| r.id == c.id));
    out.extend(rest.into_iter().map(|c| (c, false)));
    out
}

/// The note after adding: "Added 3 to Trip", or "1 added to Trip, 2 were already there".
pub(super) fn added_text(name: &str, added: u64, asked: u64) -> String {
    let there = asked.saturating_sub(added);
    match (added, there) {
        (_, 0) => format!("Added {added} to {name}"),
        (0, 1) => format!("That picture was already in {name}"),
        (0, n) => format!("All {n} were already in {name}"),
        (a, 1) => format!("{a} added to {name}, 1 was already there"),
        (a, n) => format!("{a} added to {name}, {n} were already there"),
    }
}

/// "Add photos" inside an album: the folder's pictures to choose from.
#[derive(Clone, Debug, Default)]
pub(in crate::home) struct Picker {
    pub album: Card,
    pub pics: Vec<Pic>,
    pub chosen: Vec<String>,
    pub loading: bool,
}

/// The note at the bottom; `album` gives it an "Open album" button.
#[derive(Clone, Debug)]
pub(in crate::home) struct Note {
    pub text: String,
    pub album: Option<Card>,
    /// Shows an Undo button (the last action is in the Drive's undo list).
    pub undo: bool,
    until: f64,
}

#[derive(Default)]
pub(in crate::home) struct Pick {
    /// The last square picked (Shift-click ranges start here).
    pub anchor: Option<String>,
    pub dialog: Option<Dialog>,
    pub picker: Option<Picker>,
    pub note: Option<Note>,
    /// Albums added to lately, most recent first (RECENT_KEY).
    pub recent: Option<Vec<String>>,
}

impl PhotoMode {
    fn ids(&self) -> Vec<String> {
        self.pics.iter().map(|p| p.id.clone()).collect()
    }

    /// The open album, when an album is open.
    pub(in crate::home) fn open_album(&self) -> Option<Card> {
        self.alb.open.clone().filter(|_| self.alb.tab == Tab::Albums)
    }

    /// A left click on square `i`: the selection logic, or the viewer.
    pub(super) fn clicked(&mut self, i: usize, on_box: bool, shift: bool, ctrl: bool) {
        let ids = self.ids();
        if click(&mut self.alb.selected, &mut self.pick.anchor, &ids, i, on_box, shift, ctrl) == Click::Open {
            self.viewer = Some(super::Viewer::new(i, self.pics.len()));
        }
    }

    pub(super) fn select_all(&mut self) {
        self.alb.selected = self.pics.iter().map(|p| p.id.clone()).collect();
    }

    pub(super) fn clear_selection(&mut self) {
        self.alb.selected.clear();
        self.pick.anchor = None;
    }

    /// The selected pictures in grid order.
    pub(super) fn selection(&self) -> Vec<String> {
        self.pics.iter().filter(|p| self.alb.selected.contains(&p.id)).map(|p| p.id.clone()).collect()
    }

    pub(in crate::home) fn say(&mut self, ctx: &egui::Context, text: String, album: Option<Card>) {
        self.pick.note = Some(Note { text, album, undo: false, until: ctx.input(|i| i.time) + NOTE_SECS });
        ctx.request_repaint();
    }

    /// A note with Undo (Ctrl+Z does the same).
    pub(in crate::home) fn say_undo(&mut self, ctx: &egui::Context, text: String) {
        self.say(ctx, text, None);
        if let Some(n) = &mut self.pick.note {
            n.undo = true;
        }
    }

    /// Runs `act` on `ids` (square `at` for Open).
    pub(in crate::home) fn act(&mut self, ctx: &egui::Context, store: &Backend, act: Act, ids: Vec<String>, at: Option<usize>) {
        if ids.is_empty() && act != Act::SelectAll {
            return;
        }
        match act {
            Act::Open => {
                let i = at.or_else(|| self.pics.iter().position(|p| p.id == ids[0])).unwrap_or(0);
                self.viewer = Some(super::Viewer::new(i, self.pics.len()));
            }
            Act::AddTo if !self.in_library() => {}
            Act::AddTo => {
                self.alb.boot_cards(ctx, store, &self.library_inbox);
                self.pick.dialog = Some(Dialog { ids, ..Default::default() });
            }
            Act::Favourite => self.out.push(Out::Star(ids)),
            Act::Download => {
                // A stacked picture downloads with its RAW twin.
                let ids: Vec<String> =
                    ids.iter().flat_map(|id| self.pics.iter().find(|p| &p.id == id).map_or_else(|| vec![id.clone()], |p| p.with_stack())).collect();
                download_all(&ids);
                let n = ids.len();
                self.say(ctx, if n == 1 { "Downloading 1 picture".into() } else { format!("Downloading {n} pictures") }, None);
            }
            // The viewer's own Info panel; Drive's Details only for a picture not listed here.
            Act::Info => match self.pics.iter().position(|p| p.id == ids[0]) {
                Some(i) => {
                    let mut v = super::Viewer::new(i, self.pics.len());
                    v.info = true;
                    self.viewer = Some(v);
                }
                None => self.out.push(Out::Info(ids[0].clone())),
            },
            Act::Lock => self.lk.ask = Some(ids),
            Act::Unlock => self.move_locked(ctx, store, ids, false),
            Act::Delete if self.locked_tab() => self.say(ctx, "Move pictures out of the Locked folder first".into(), None),
            Act::Delete => {
                // Deleting a stack moves both files (JPEG and RAW) to the bin, which Undo restores.
                let ids: Vec<String> =
                    ids.iter().flat_map(|id| self.pics.iter().find(|p| &p.id == id).map_or_else(|| vec![id.clone()], |p| p.with_stack())).collect();
                self.pics.retain(|p| !ids.contains(&p.id));
                if let Some(v) = &mut self.viewer {
                    v.fit_list(self.pics.len());
                }
                self.clear_selection();
                self.out.push(Out::Delete(ids));
            }
            Act::SelectAll => self.select_all(),
            Act::RotateLeft | Act::RotateRight => self.rotate(ctx, store, ids, act == Act::RotateLeft),
            Act::Remove => {
                let Some(c) = self.open_album() else { return };
                post(ctx, store, &self.library_inbox, &format!("api/photos/albums/{}/remove", c.id), json!({ "files": ids }));
                let n = ids.len();
                self.pics.retain(|p| !ids.contains(&p.id));
                self.clear_selection();
                self.out.push(Out::Did(format!("Removed from {}", c.name), vec![crate::home::undo::Step::AlbumRemove { album: c.id.clone(), ids }]));
                self.say_undo(ctx, format!("Removed {n} from {}", c.name));
            }
        }
    }

    /// Adds `ids` to album `c`; the server's answer brings the note.
    pub(super) fn add_to(&mut self, ctx: &egui::Context, store: &Backend, c: &Card, ids: Vec<String>) {
        let (id, name, asked) = (c.id.clone(), c.name.clone(), ids.len() as u64);
        self.used(store, &c.id);
        let (inbox, ctx2) = (self.library_inbox.clone(), ctx.clone());
        store.call(
            ctx,
            Method::Post,
            &format!("api/photos/albums/{}/add", enc(&c.id)),
            Some(json!({ "files": ids }).to_string().into_bytes()),
            Box::new(move |a| PhotoMode::push(&inbox, &ctx2, answer(a, id, name, asked))),
        );
        self.clear_selection();
    }

    /// A new album named `name`, with `ids` in it.
    pub(super) fn new_album(&mut self, ctx: &egui::Context, store: &Backend, name: &str, ids: Vec<String>) {
        let (inbox, ctx2, asked) = (self.library_inbox.clone(), ctx.clone(), ids.len() as u64);
        store.call(
            ctx,
            Method::Post,
            "api/photos/albums",
            Some(json!({ "name": name, "files": ids }).to_string().into_bytes()),
            Box::new(move |a| PhotoMode::push(&inbox, &ctx2, answer(a, String::new(), String::new(), asked))),
        );
        self.clear_selection();
    }

    /// The server added pictures to an album: the note, and the album list again.
    pub(super) fn added(&mut self, ctx: &egui::Context, store: &Backend, c: Card, n: u64, asked: u64, ids: Vec<String>) {
        let text = added_text(&c.name, n, asked);
        self.used(store, &c.id);
        let undo = !ids.is_empty();
        if undo {
            self.out.push(Out::Did(format!("Added to {}", c.name), vec![crate::home::undo::Step::AlbumAdd { album: c.id.clone(), ids }]));
        }
        self.say(ctx, text, Some(c));
        if let Some(note) = &mut self.pick.note {
            note.undo = undo;
        }
        self.alb.changed(ctx, store, &self.library_inbox);
    }

    fn recent(&mut self, store: &Backend) -> &mut Vec<String> {
        self.pick
            .recent
            .get_or_insert_with(|| store.setting(RECENT_KEY).unwrap_or_default().split(',').filter(|x| !x.is_empty()).map(str::to_string).collect())
    }

    /// Album `id` was just used: first under Recent.
    fn used(&mut self, store: &Backend, id: &str) {
        let r = self.recent(store);
        r.retain(|x| x != id);
        r.insert(0, id.to_string());
        r.truncate(RECENT * 2);
        let v = r.join(",");
        store.set_setting(RECENT_KEY, &v);
    }

    /// The bar that replaces the top bar while pictures are selected.
    pub(super) fn sel_bar_ui(&mut self, ui: &mut Ui, store: &Backend) {
        let ctx = ui.ctx().clone();
        let mut chosen = None;
        ui.horizontal(|ui| {
            ui.set_min_height(36.0);
            if house::close_button(ui) {
                chosen = Some(None);
            }
            let n = self.alb.selected.len();
            ui.label(RichText::new(format!("{n} selected")).size(house::BODY).strong().color(theme::text()));
            ui.add_space(house::L);
            let all = ui.add(house::quiet("Select all"));
            house::target(&ctx, "photos:sel:select-all", all.rect);
            if all.clicked() {
                chosen = Some(Some(Act::SelectAll));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = house::S;
                let mut rows = sel_rows(self.open_album().is_some());
                if !self.in_library() {
                    rows.retain(|(act, _, _, _)| *act != Act::AddTo);
                }
                let more = house::icon_button(ui, "more_vert", "More", false);
                house::target(&ctx, "photos:sel:more", more.rect);
                egui::Popup::menu(&more).show(|ui| {
                    house::menu(ui, 220.0);
                    for &(act, icon, label, _) in rows.iter().filter(|r| !r.3) {
                        if house::menu_row(ui, Some(icon), label, "", true) {
                            chosen = Some(Some(act));
                        }
                    }
                });
                for &(act, icon, label, _) in rows.iter().filter(|r| r.3).rev() {
                    let r = ui.add(house::quiet_with(Some(icon), label, false));
                    house::target(&ctx, &format!("photos:sel:{}", label.to_lowercase().replace(' ', "-")), r.rect);
                    if r.clicked() {
                        chosen = Some(Some(act));
                    }
                }
            });
        });
        match chosen {
            Some(None) => self.clear_selection(),
            Some(Some(act)) => {
                let ids = self.selection();
                self.act(&ctx, store, act, ids, None);
            }
            None => {}
        }
    }

    /// "Add photos" in the open album: lists the Photos folder (with subfolders) to choose from.
    pub(super) fn open_picker(&mut self, ctx: &egui::Context, store: &Backend) {
        let Some(album) = self.open_album() else { return };
        let folder = self.folder.clone().unwrap_or_default();
        let url = format!("api/photos?folder={}&sort=taken&desc=1&sub=1", enc(&folder));
        get(ctx, store, &self.library_inbox, &url, Got::Picker);
        self.pick.picker = Some(Picker { album, loading: true, ..Default::default() });
    }

    /// The dialog, the picker and the note: over everything (the viewer too).
    pub(in crate::home) fn overlays(&mut self, ctx: &egui::Context, store: &Backend) {
        self.dialog_ui(ctx, store);
        self.locked_dialog(ctx, store);
        self.picker_ui(ctx, store);
        self.note_ui(ctx, store);
    }

    fn dialog_ui(&mut self, ctx: &egui::Context, store: &Backend) {
        let Some(mut d) = self.pick.dialog.take() else { return };
        let skip = self.open_album().map(|c| c.id);
        let albums: Vec<Card> = self.alb.albums.iter().filter(|c| Some(&c.id) != skip.as_ref()).cloned().collect();
        let recent = self.recent(store).clone();
        let rows = picker_rows(&albums, &recent, &d.search);
        // ↑/↓ move over "New album" (row 0) and the albums; Enter picks.
        let last = rows.len();
        let (up, down, enter) =
            ctx.input(|i| (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::Enter)));
        if !d.naming {
            if down {
                d.cursor = (d.cursor + 1).min(last);
            }
            if up {
                d.cursor = d.cursor.saturating_sub(1);
            }
            d.cursor = d.cursor.min(last);
        }
        let mut close = false;
        let (mut into, mut create) = (None, None);
        house::dialog(ctx, "photos_add_to_album", 360.0, "Add to album", Some(&mut close), |ui| {
            if d.naming {
                let r = ui.add(house::field(&mut d.text, house::BODY).hint_text("Album name").desired_width(f32::INFINITY));
                if !d.focused {
                    r.request_focus();
                    d.focused = true;
                }
                let enter = r.lost_focus() && enter;
                ui.add_space(house::L);
                ui.horizontal(|ui| {
                    let ok = !d.text.trim().is_empty();
                    let b = if ok { house::primary("Create") } else { house::primary_off("Create") };
                    if (ui.add(b).clicked() || enter) && ok {
                        create = Some(d.text.trim().to_string());
                    }
                    if ui.add(house::secondary("Back")).clicked() {
                        d.naming = false;
                        d.focused = false;
                    }
                });
                return;
            }
            let before = d.search.clone();
            let r = ui.add(house::field(&mut d.search, house::BODY).hint_text("Search albums").desired_width(f32::INFINITY));
            if !d.focused {
                r.request_focus();
                d.focused = true;
            }
            // Typing: the first match is the one Enter takes.
            if d.search != before {
                d.cursor = usize::from(!d.search.trim().is_empty() && last > 0);
            }
            ui.add_space(house::S);
            ui.vertical(|ui| {
                house::menu(ui, 336.0);
                let hl = |ui: &mut Ui, on: bool| {
                    if on {
                        let r = ui.available_rect_before_wrap();
                        ui.painter().rect_filled(Rect::from_min_size(r.min, vec2(r.width(), house::ROW_HEIGHT)), 0, theme::menu_hover());
                    }
                };
                hl(ui, d.cursor == 0);
                if house::menu_row(ui, Some("add"), "New album", "", true) || (enter && d.cursor == 0) {
                    d.naming = true;
                    d.focused = false;
                    d.text = d.search.trim().to_string();
                }
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    let mut heading = None;
                    for (k, (c, rec)) in rows.iter().enumerate() {
                        if heading != Some(*rec) {
                            heading = Some(*rec);
                            ui.add_space(house::XS);
                            house::caps(
                                ui,
                                if *rec {
                                    "Recent"
                                } else if d.search.trim().is_empty() {
                                    "All albums"
                                } else {
                                    "Albums"
                                },
                            );
                        }
                        hl(ui, d.cursor == k + 1);
                        let n = format!("{}", c.count);
                        if house::menu_row(ui, Some("photo_library"), &c.name, &n, true) || (enter && d.cursor == k + 1) {
                            into = Some(c.clone());
                        }
                    }
                    if rows.is_empty() {
                        house::note(ui, if albums.is_empty() { "No albums yet." } else { "No album has that name." });
                    }
                });
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        if let Some(c) = into {
            self.add_to(ctx, store, &c, d.ids);
        } else if let Some(name) = create {
            self.new_album(ctx, store, &name, d.ids);
        } else if !close {
            self.pick.dialog = Some(d);
        }
    }

    fn picker_ui(&mut self, ctx: &egui::Context, store: &Backend) {
        let Some(mut p) = self.pick.picker.take() else { return };
        let have: HashSet<String> = self.pics.iter().map(|x| x.id.clone()).collect();
        let mut close = false;
        let mut add = false;
        let title = format!("Add photos to {}", p.album.name);
        // Thumbnails are asked for before the dialog borrows the picker.
        for x in p.pics.iter().take(120) {
            self.want_id(ctx, store, &x.id, x.v.as_deref(), SMALL);
        }
        let frame = self.frame;
        let tex: Vec<Option<egui::TextureHandle>> = p.pics.iter().map(|x| self.tex.get(&(x.id.clone(), SMALL), frame)).collect();
        let mut cancel = false;
        house::dialog(ctx, "photos_add_photos", 4.0 * (PICK_TILE + 4.0) + 24.0, &title, Some(&mut close), |ui| {
            if p.loading {
                house::note(ui, "Loading…");
            } else if p.pics.iter().all(|x| have.contains(&x.id)) {
                house::note(ui, "Every picture here is in the album already.");
            }
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    for (x, t) in p.pics.iter().zip(&tex) {
                        // Already in the album: ticked, and can't be taken off here.
                        let locked = have.contains(&x.id);
                        let (rect, resp) = ui.allocate_exact_size(vec2(PICK_TILE, PICK_TILE), Sense::click());
                        let painter = ui.painter_at(rect);
                        painter.rect_filled(rect, 0, Color32::from_rgb(0xE8, 0xEA, 0xED));
                        if let Some(t) = t {
                            let [w, h] = t.size();
                            painter.image(t.id(), rect, super::cover_uv(w as f32, h as f32), Color32::WHITE);
                        }
                        let on = locked || p.chosen.contains(&x.id);
                        check(&painter, rect, on, true);
                        if locked {
                            painter.rect_filled(rect, 0, Color32::from_white_alpha(110));
                        }
                        let tip = if locked { format!("{}\nAlready in the album", x.name) } else { x.name.clone() };
                        if resp.on_hover_text(tip).clicked() && !locked {
                            if on {
                                p.chosen.retain(|c| c != &x.id);
                            } else {
                                p.chosen.push(x.id.clone());
                            }
                        }
                    }
                });
            });
            ui.add_space(house::L);
            ui.horizontal(|ui| {
                let n = p.chosen.len();
                let label = if n == 0 { "Add".to_string() } else { format!("Add {n}") };
                let b = if n > 0 { house::primary(&label) } else { house::primary_off(&label) };
                if ui.add(b).clicked() && n > 0 {
                    add = true;
                }
                if ui.add(house::secondary("Cancel")).clicked() {
                    cancel = true;
                }
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        close |= cancel;
        if add {
            let ids = std::mem::take(&mut p.chosen);
            self.add_to(ctx, store, &p.album, ids);
        } else if !close {
            self.pick.picker = Some(p);
        }
    }

    pub(super) fn take_picker(&mut self, v: &Json) {
        if let Some(p) = &mut self.pick.picker {
            p.loading = false;
            p.pics = Self::parse_pics(v);
        }
    }

    fn note_ui(&mut self, ctx: &egui::Context, store: &Backend) {
        let now = ctx.input(|i| i.time);
        let Some(note) = self.pick.note.clone() else { return };
        if now > note.until {
            self.pick.note = None;
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64((note.until - now).max(0.05)));
        let screen = ctx.content_rect();
        let (mut open, mut closed) = (false, false);
        egui::Area::new(egui::Id::unique("photos_note"))
            .order(egui::Order::Tooltip)
            .fixed_pos(egui::pos2(screen.left() + house::XL, screen.bottom() - house::XL - 48.0))
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(house::TIP_BG)
                    .corner_radius(house::PANEL_RADIUS)
                    .inner_margin(egui::Margin { left: 16, right: 8, top: 6, bottom: 6 })
                    .show(ui, |ui| {
                        ui.set_min_height(36.0);
                        house::view_bar_style(ui);
                        ui.horizontal_centered(|ui| {
                            ui.label(RichText::new(&note.text).size(house::BODY).color(Color32::WHITE));
                            ui.add_space(house::L);
                            if note.undo && ui.add(house::quiet_in("Undo", theme::share())).clicked() {
                                self.pick.note = None;
                                self.out.push(Out::Undo);
                            }
                            if note.album.is_some() && ui.add(house::quiet_in("Open album", theme::share())).clicked() {
                                open = true;
                            }
                            let x = house::quiet_in("\u{2715}", Color32::WHITE);
                            if ui.add(x).on_hover_text("Close").clicked() {
                                closed = true;
                            }
                        });
                    });
            });
        if open && let Some(c) = note.album {
            self.pick.note = None;
            self.open_album_card(ctx, store, c);
        } else if closed {
            self.pick.note = None;
        }
    }

    /// Goes to Albums and opens `c` (the note's "Open album").
    pub(super) fn open_album_card(&mut self, ctx: &egui::Context, store: &Backend, c: Card) {
        self.viewer = None;
        self.editor = None;
        self.go(Tab::Albums);
        let c = self.alb.albums.iter().find(|x| x.id == c.id).cloned().unwrap_or(c);
        self.open_card(ctx, store, c);
    }
}

impl PhotoMode {
    /// A dialog of Photos' has the keys (the Drive's own keys wait).
    pub(in crate::home) fn pick_busy(&self) -> bool {
        self.pick.dialog.is_some() || self.pick.picker.is_some()
    }
}

/// The test hook (TESTING.md): the same paths as the mouse.
#[cfg(target_arch = "wasm32")]
impl Pick {
    pub(super) fn test_state(&self) -> Json {
        json!({
            "anchor": self.anchor,
            "dialog": self.dialog.as_ref().map(|d| json!({"ids": d.ids, "naming": d.naming, "text": d.text, "search": d.search, "cursor": d.cursor})),
            "picker": self.picker.as_ref().map(|p| json!({"album": p.album.id, "loading": p.loading, "count": p.pics.len(), "chosen": p.chosen})),
            "note": self.note.as_ref().map(|n| json!({"text": n.text, "album": n.album.as_ref().map(|c| &c.id)})),
            "recent": self.recent,
        })
    }
}

#[cfg(target_arch = "wasm32")]
impl PhotoMode {
    /// `home:photos:click` {index, box, shift, ctrl}, `home:photos:menu` {index, act: open, addto,
    /// favourite, download, info, delete, remove}, `home:photos:bar` {act (+ selectall)},
    /// `home:photos:addto` {album} or {new: name}, `home:photos:picker` {open | choose: [index] | add},
    /// `home:photos:note:open`.
    pub(in crate::home) fn pick_test(&mut self, ctx: &egui::Context, store: &Backend, id: &str, p: &Json) -> Result<(), String> {
        let flag = |k: &str| p.get(k).and_then(Json::as_bool).unwrap_or(false);
        let index = || p.get("index").and_then(Json::as_u64).map(|i| i as usize).ok_or("index is missing");
        let act = || -> Result<Act, String> {
            Ok(match p.get("act").and_then(Json::as_str).unwrap_or("") {
                "open" => Act::Open,
                "addto" => Act::AddTo,
                "favourite" => Act::Favourite,
                "download" => Act::Download,
                "info" => Act::Info,
                "delete" => Act::Delete,
                "lock" => Act::Lock,
                "unlock" => Act::Unlock,
                "remove" => Act::Remove,
                "selectall" => Act::SelectAll,
                a => return Err(format!("unknown act `{a}`")),
            })
        };
        match id {
            "home:photos:click" => {
                let i = index()?;
                if i >= self.pics.len() {
                    return Err("no such picture".into());
                }
                self.clicked(i, flag("box"), flag("shift"), flag("ctrl"));
            }
            "home:photos:menu" => {
                let i = index()?;
                let ids: Vec<String> = self.pics.iter().map(|x| x.id.clone()).collect();
                let t = targets(&mut self.alb.selected, &mut self.pick.anchor, &ids, i);
                let a = act()?;
                if a == Act::Remove && self.open_album().is_none() {
                    return Err("Remove from album is only in an album".into());
                }
                self.act(ctx, store, a, t, Some(i));
            }
            "home:photos:bar" => {
                let ids = self.selection();
                self.act(ctx, store, act()?, ids, None);
            }
            "home:photos:addto" => {
                let d = self.pick.dialog.take().ok_or("Add to album isn't open")?;
                if let Some(n) = p.get("new").and_then(Json::as_str) {
                    self.new_album(ctx, store, n, d.ids);
                } else {
                    let k = p.get("album").and_then(Json::as_str).unwrap_or("");
                    let c = self.alb.albums.iter().find(|c| c.id == k || c.name == k).cloned().ok_or("no such album")?;
                    self.add_to(ctx, store, &c, d.ids);
                }
            }
            "home:photos:picker" => {
                if flag("open") {
                    self.open_picker(ctx, store);
                } else if flag("add") {
                    let mut pk = self.pick.picker.take().ok_or("the picker isn't open")?;
                    let ids = std::mem::take(&mut pk.chosen);
                    self.add_to(ctx, store, &pk.album, ids);
                } else {
                    let have: HashSet<String> = self.pics.iter().map(|x| x.id.clone()).collect();
                    let pk = self.pick.picker.as_mut().ok_or("the picker isn't open")?;
                    for i in p.get("choose").and_then(Json::as_array).into_iter().flatten().filter_map(Json::as_u64) {
                        let x = pk.pics.get(i as usize).ok_or("no such picture")?;
                        if !have.contains(&x.id) && !pk.chosen.contains(&x.id) {
                            pk.chosen.push(x.id.clone());
                        }
                    }
                }
            }
            "home:photos:note:open" => {
                let c = self.pick.note.take().and_then(|n| n.album).ok_or("no note with an album")?;
                self.open_album_card(ctx, store, c);
            }
            _ => return Err(format!("unknown command `{id}`")),
        }
        Ok(())
    }
}

/// The server's answer to an add or a new album, as an `Added` (or the error).
fn answer(a: Result<(u16, Vec<u8>), String>, id: String, name: String, asked: u64) -> Got {
    match a {
        Ok((200, b)) => {
            let v: Json = serde_json::from_slice(&b).unwrap_or_default();
            let n = v["added"].as_u64().or(v["count"].as_u64()).unwrap_or(0);
            let id = v["id"].as_str().map_or(id, str::to_string);
            let name = v["name"].as_str().map_or(name, str::to_string);
            let ids = v["ids"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect();
            Got::Added(Card { id, name, ..Default::default() }, n, asked, ids)
        }
        Ok((s, b)) => {
            let v: Json = serde_json::from_slice(&b).unwrap_or_default();
            Got::Error(v["error"].as_str().map_or(format!("The server said {s}."), str::to_string))
        }
        Err(e) => Got::Error(e),
    }
}

/// The round checkbox at a square's top left: white ring when off, blue tick when on.
pub(super) fn check(painter: &egui::Painter, rect: Rect, on: bool, show: bool) {
    if on {
        painter.rect_filled(rect, 0, Color32::from_rgba_unmultiplied(0x1A, 0x73, 0xE8, 60));
    }
    if !(on || show) {
        return;
    }
    let c = box_rect(rect).center();
    painter.circle(c, 10.0, if on { super::super::ACCENT } else { Color32::from_black_alpha(60) }, egui::Stroke::new(1.5, Color32::WHITE));
    if on {
        painter.text(c, egui::Align2::CENTER_CENTER, "✓", egui::FontId::proportional(14.0), Color32::WHITE);
    }
}

/// Where a square's checkbox takes clicks.
pub(super) fn box_rect(rect: Rect) -> Rect {
    Rect::from_center_size(rect.left_top() + vec2(16.0, 16.0), vec2(28.0, 28.0))
}

/// Downloads the pictures: one picture as the plain original, several as ONE .zip (the server's
/// `api/zip`, as in Drive).
fn download_all(ids: &[String]) {
    download(&download_url(ids));
}

fn download_url(ids: &[String]) -> String {
    match ids {
        [one] => format!("api/files/{one}"),
        _ => format!("api/zip?files={}&folders=", ids.join(",")),
    }
}

/// Starts a download of `href` (the server sends it as an attachment): an `<a download>` click.
fn download(href: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        if let Some(doc) = web_sys::window().and_then(|w| w.document())
            && let Ok(a) = doc.create_element("a")
            && let Ok(a) = a.dyn_into::<web_sys::HtmlAnchorElement>()
        {
            a.set_href(href);
            a.set_download("");
            a.click();
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = href;
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The selection bar shows four commands; the rest (Remove from album, Info) are under More.
    #[test]
    fn the_selection_bar_shows_four_then_more() {
        for in_album in [false, true] {
            let rows = sel_rows(in_album);
            let shown: Vec<_> = rows.iter().filter(|r| r.3).map(|r| r.2).collect();
            assert_eq!(shown, ["Add to album", "Favourite", "Download", "Move to bin"]);
            let more: Vec<_> = rows.iter().filter(|r| !r.3).map(|r| r.2).collect();
            assert_eq!(more, if in_album { vec!["Remove from album", "Info"] } else { vec!["Info"] });
            for r in rows {
                assert!(crate::icons::icon(r.1).is_some(), "{}", r.1);
            }
        }
    }

    fn ids(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("p{i}")).collect()
    }

    fn set(v: &[&str]) -> HashSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// Every kind of click, from a given selection and anchor: what it does and what's left selected.
    #[test]
    fn several_pictures_download_as_one_zip() {
        assert_eq!(download_url(&ids(1)), "api/files/p0");
        assert_eq!(download_url(&ids(3)), "api/zip?files=p0,p1,p2&folders=");
    }

    #[test]
    fn clicks_select_like_google_photos() {
        // (selected before, anchor, square, on_box, shift, ctrl) → (result, selected after, anchor after)
        type Row = (&'static [&'static str], Option<&'static str>, usize, bool, bool, bool, Click, &'static [&'static str], Option<&'static str>);
        let rows: &[Row] = &[
            // Plain click, nothing selected: opens; nothing changes.
            (&[], None, 2, false, false, false, Click::Open, &[], None),
            // The checkbox selects, and starts a range there.
            (&[], None, 2, true, false, false, Click::Picked, &["p2"], Some("p2")),
            // Ctrl-click toggles on, and off.
            (&[], None, 1, false, false, true, Click::Picked, &["p1"], Some("p1")),
            (&["p1", "p3"], Some("p3"), 1, false, false, true, Click::Picked, &["p3"], Some("p1")),
            // While something is selected, a plain click toggles (it never opens).
            (&["p0"], Some("p0"), 4, false, false, false, Click::Picked, &["p0", "p4"], Some("p4")),
            (&["p0", "p4"], Some("p4"), 4, false, false, false, Click::Picked, &["p0"], Some("p4")),
            // The checkbox on a selected square takes it off.
            (&["p2"], Some("p2"), 2, true, false, false, Click::Picked, &[], Some("p2")),
            // Shift adds the range from the anchor, forwards and backwards, keeping the anchor.
            (&["p1"], Some("p1"), 4, false, true, false, Click::Picked, &["p1", "p2", "p3", "p4"], Some("p1")),
            (&["p4"], Some("p4"), 2, false, true, false, Click::Picked, &["p2", "p3", "p4"], Some("p4")),
            (&["p0", "p5"], Some("p5"), 3, false, true, false, Click::Picked, &["p0", "p3", "p4", "p5"], Some("p5")),
            // Shift with no anchor (or one no longer listed) picks just that square.
            (&[], None, 3, false, true, false, Click::Picked, &["p3"], Some("p3")),
            (&[], Some("gone"), 3, false, true, false, Click::Picked, &["p3"], Some("p3")),
            // Past the end: nothing happens.
            (&["p1"], Some("p1"), 99, false, false, false, Click::Picked, &["p1"], Some("p1")),
        ];
        let list = ids(6);
        for (k, (before, anchor, i, on_box, shift, ctrl, want, after, want_anchor)) in rows.iter().enumerate() {
            let mut sel = set(before);
            let mut a = anchor.map(str::to_string);
            let got = click(&mut sel, &mut a, &list, *i, *on_box, *shift, *ctrl);
            assert_eq!(got, *want, "row {k}");
            assert_eq!(sel, set(after), "row {k}");
            assert_eq!(a.as_deref(), *want_anchor, "row {k}");
        }
    }

    #[test]
    fn right_click_acts_on_the_selection_or_just_that_picture() {
        let list = ids(5);
        // In the selection: all of it, in grid order; the selection stays.
        let (mut sel, mut a) = (set(&["p3", "p1"]), Some("p3".to_string()));
        assert_eq!(targets(&mut sel, &mut a, &list, 1), ["p1", "p3"]);
        assert_eq!(sel, set(&["p1", "p3"]));
        // Outside it: that picture alone becomes the selection.
        assert_eq!(targets(&mut sel, &mut a, &list, 4), ["p4"]);
        assert_eq!((sel, a.as_deref()), (set(&["p4"]), Some("p4")));
        // Nothing selected: the same.
        let (mut sel, mut a) = (HashSet::new(), None);
        assert_eq!(targets(&mut sel, &mut a, &list, 0), ["p0"]);
    }

    /// Every menu row, in and out of an album: label, icon that exists, and order.
    #[test]
    fn the_menu_has_every_command() {
        for (in_album, want) in [
            (
                false,
                vec![
                    "Open",
                    "Add to album…",
                    "Favourite",
                    "Download",
                    "Rotate left",
                    "Rotate right",
                    "Info",
                    "Move to Locked folder…",
                    "Move to bin",
                ],
            ),
            (
                true,
                vec![
                    "Open",
                    "Add to album…",
                    "Favourite",
                    "Download",
                    "Rotate left",
                    "Rotate right",
                    "Info",
                    "Remove from album",
                    "Move to Locked folder…",
                    "Move to bin",
                ],
            ),
        ] {
            let rows = menu_rows(in_album, false);
            assert_eq!(rows.iter().map(|r| r.2).collect::<Vec<_>>(), want);
            for (act, icon, label) in rows {
                assert!(crate::icons::icon(icon).is_some(), "{label}: no icon `{icon}`");
                assert_eq!(act == Act::Remove, label == "Remove from album");
            }
        }
    }

    #[test]
    fn the_server_answer_becomes_the_note() {
        let ok = |b: Json| answer(Ok((200, b.to_string().into_bytes())), "a1".into(), "Trip".into(), 4);
        // An add: "added" counts only the new ones.
        match ok(json!({"id": "a1", "name": "Trip", "count": 5, "added": 2})) {
            Got::Added(c, n, asked, _) => assert_eq!((c.id.as_str(), c.name.as_str(), n, asked), ("a1", "Trip", 2, 4)),
            _ => panic!("not added"),
        }
        // A new album: its count is how many went in.
        match answer(Ok((200, json!({"id": "b2", "name": "New", "count": 3}).to_string().into_bytes())), String::new(), String::new(), 3) {
            Got::Added(c, n, _, _) => assert_eq!((c.id.as_str(), c.name.as_str(), n), ("b2", "New", 3)),
            _ => panic!("not added"),
        }
        match answer(Ok((400, br#"{"error":"Too many"}"#.to_vec())), String::new(), String::new(), 1) {
            Got::Error(e) => assert_eq!(e, "Too many"),
            _ => panic!("not an error"),
        }
    }

    fn card(id: &str, name: &str) -> Card {
        Card { id: id.into(), name: name.into(), ..Default::default() }
    }

    #[test]
    fn the_picker_lists_recent_then_a_to_z_and_searches() {
        let all = [card("1", "whitby 2026"), card("2", "Beach"), card("3", "Zoo"), card("4", "Cats"), card("5", "Walks")];
        let names = |v: Vec<(Card, bool)>| v.into_iter().map(|(c, r)| format!("{}{}", c.name, if r { "*" } else { "" })).collect::<Vec<_>>();
        // (recent, search) → rows (* = under Recent)
        let rows: &[(&[&str], &str, &[&str])] = &[
            (&[], "", &["Beach", "Cats", "Walks", "whitby 2026", "Zoo"]),
            (&["3", "1"], "", &["Zoo*", "whitby 2026*", "Beach", "Cats", "Walks"]),
            // A forgotten (deleted) album in Recent is skipped.
            (&["gone", "2"], "", &["Beach*", "Cats", "Walks", "whitby 2026", "Zoo"]),
            // Search: any case, anywhere in the name, A–Z, no Recent heading.
            (&["3"], "WH", &["whitby 2026"]),
            (&["3"], "a", &["Beach", "Cats", "Walks"]),
            (&[], "nothing", &[]),
        ];
        for (recent, q, want) in rows {
            let recent: Vec<String> = recent.iter().map(|s| s.to_string()).collect();
            assert_eq!(names(picker_rows(&all, &recent, q)), *want, "recent {recent:?} search {q:?}");
        }
        // At most RECENT under Recent.
        let recent: Vec<String> = ["1", "2", "3", "4", "5"].iter().chain(&["1"]).map(|s| s.to_string()).collect();
        assert_eq!(picker_rows(&all, &recent, "").iter().filter(|r| r.1).count(), RECENT);
    }

    #[test]
    fn the_note_says_what_was_already_there() {
        for (added, asked, want) in [
            (3, 3, "Added 3 to Trip"),
            (1, 1, "Added 1 to Trip"),
            (0, 1, "That picture was already in Trip"),
            (0, 4, "All 4 were already in Trip"),
            (2, 3, "2 added to Trip, 1 was already there"),
            (3, 5, "3 added to Trip, 2 were already there"),
        ] {
            assert_eq!(added_text("Trip", added, asked), want);
        }
    }
}
