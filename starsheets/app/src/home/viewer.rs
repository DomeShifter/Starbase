//! The full-screen photo viewer and slideshow (Photos job 2), over the grid in photos.rs.
//!
//! - Arrows (keys, buttons, or a swipe/drag sideways) go to the next or previous picture; Esc
//!   closes; Space plays or pauses the slideshow; + and − (or the wheel, or a pinch) zoom; 0 fits;
//!   a double click zooms in or back.
//! - Shown at 1024 px first (the 256 px one while that comes). The original is asked for only
//!   when zoomed in past what 1024 px can show, no bigger than the GPU's largest texture.
//! - The next and previous pictures are asked for ahead of time.
//! - Slideshow: every 3, 5 or 10 seconds, in order or shuffled.
//!
//! - Edit (or E) opens the editor (photo_edit.rs) over the viewer; it takes every key while open.
//!
//! Black in light and dark mode alike, like other photo viewers: the picture is what matters.

use egui::{Color32, Key, Modifiers, Order, Rect, RichText, Sense, Vec2, pos2, vec2};

use super::photos::{LARGE, ORIGINAL, PhotoMode, SCREEN, SMALL, date_label};
use crate::house;
use crate::store::Backend;
use serde_json::{Value as Json, json};

/// Slideshow intervals offered (seconds).
pub(super) const EVERY: [f64; 3] = [3.0, 5.0, 10.0];
const MAX_ZOOM: f32 = 8.0;
/// A sideways drag at least this long (points) goes to the next or previous picture.
const SWIPE: f32 = 60.0;

pub(super) struct Viewer {
    /// Where in [`Self::order`] we are.
    pub pos: usize,
    /// The order pictures are shown in: the grid's, or shuffled.
    pub order: Vec<usize>,
    /// 1 = the whole picture fits the screen.
    pub zoom: f32,
    pub pan: Vec2,
    pub playing: bool,
    pub shuffle: bool,
    /// The Info panel (ⓘ, or I) is open.
    pub info: bool,
    /// Showing the stacked RAW file instead of the JPEG.
    pub raw: bool,
    /// The "Open with" menu is showing.
    pub menu: bool,
    pub every: f64,
    pub next_at: f64,
    /// A sideways drag so far (not zoomed in).
    drag: f32,
    /// Which texture was shown last frame ("256", "1024", "original" or "none"), for tests.
    pub showing: &'static str,
}

/// A fresh order: 0..n, or shuffled from `seed` with `first` first.
pub(super) fn order(n: usize, shuffle: bool, first: usize, seed: u64) -> Vec<usize> {
    let mut o: Vec<usize> = (0..n).collect();
    if shuffle && n > 1 {
        let mut s = seed | 1;
        for i in (1..n).rev() {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            o.swap(i, (s % (i as u64 + 1)) as usize);
        }
        if let Some(k) = o.iter().position(|&x| x == first) {
            o.swap(0, k);
        }
    }
    o
}

/// What a finished sideways drag of `dx` points does: -1 back, 1 on, 0 nothing.
pub(super) fn swipe(dx: f32) -> i32 {
    if dx <= -SWIPE {
        1
    } else if dx >= SWIPE {
        -1
    } else {
        0
    }
}

impl Viewer {
    pub fn new(index: usize, n: usize) -> Viewer {
        Viewer {
            pos: index.min(n.saturating_sub(1)),
            order: (0..n).collect(),
            zoom: 1.0,
            pan: Vec2::ZERO,
            playing: false,
            shuffle: false,
            info: false,
            raw: false,
            menu: false,
            every: EVERY[1],
            next_at: 0.0,
            drag: 0.0,
            showing: "none",
        }
    }

    /// The picture shown (its place in the grid).
    pub fn index(&self) -> usize {
        self.order.get(self.pos).copied().unwrap_or(0)
    }

    /// The listing changed size: keep within it.
    pub fn fit_list(&mut self, n: usize) {
        if self.order.len() != n {
            let at = self.index().min(n.saturating_sub(1));
            self.order = order(n, false, at, 0);
            self.pos = at;
            self.shuffle = false;
        }
    }

    /// `by` pictures on (or back), going round at the ends.
    pub fn step(&mut self, by: i32) {
        let n = self.order.len() as i64;
        if n == 0 {
            return;
        }
        self.pos = (self.pos as i64 + by as i64).rem_euclid(n) as usize;
        self.zoom = 1.0;
        self.pan = Vec2::ZERO;
        self.raw = false;
    }

    pub fn play(&mut self, now: f64) {
        self.playing = true;
        self.next_at = now + self.every;
    }

    pub fn set_shuffle(&mut self, on: bool, seed: u64) {
        let at = self.index();
        self.shuffle = on;
        self.order = order(self.order.len(), on, at, seed);
        self.pos = self.order.iter().position(|&x| x == at).unwrap_or(0);
    }

    /// The slideshow's clock: true when it moved on.
    pub fn tick(&mut self, now: f64) -> bool {
        if self.playing && now >= self.next_at {
            self.step(1);
            self.next_at = now + self.every;
            return true;
        }
        false
    }

    /// Escape peels one layer, like Google Photos: the Open with menu, then the
    /// Info panel, and only then the viewer itself. Returns false when the viewer
    /// should close.
    pub fn escape(&mut self) -> bool {
        if self.menu {
            self.menu = false;
        } else if self.info {
            self.info = false;
        } else {
            return false;
        }
        true
    }

    fn zoom_by(&mut self, f: f32) {
        self.zoom = (self.zoom * f).clamp(1.0, MAX_ZOOM);
        if self.zoom == 1.0 {
            self.pan = Vec2::ZERO;
        }
    }
}

/// Keys, buttons, drags and drawing; false when the viewer was closed.
/// The Info panel's width: the picture shrinks to the left of it.
const INFO_W: f32 = 320.0;

pub(super) fn ui(pm: &mut PhotoMode, ctx: &egui::Context, store: &Backend) -> bool {
    if pm.viewer.is_none() || pm.pics.is_empty() {
        pm.viewer = None;
        pm.editor = None;
        super::video::hide();
        return false;
    }
    if pm.editor.is_some() {
        super::video::hide();
        super::photo_edit::ui(pm, ctx, store);
        return true;
    }
    let Some(mut v) = pm.viewer.take() else { return false };
    v.fit_list(pm.pics.len());
    let now = ctx.input(|i| i.time);
    let mut open = true;
    let mut edit = false;
    let mut menu_at: Option<Rect> = None;
    // An album clicked in the Info panel: opened once the viewer is closed.
    let mut goto: Option<super::photos::albums::Card> = None;
    // A person clicked in the Info panel (Some(None): "unnamed faces", the People page).
    let mut person: Option<Option<super::photos::albums::Card>> = None;
    // What the person asked for on this picture (keys or buttons): L add to album, Shift-L out of
    // the open album, F favourite, Delete move to bin. Run after drawing, on the picture shown.
    let mut acts: Vec<super::photos::Act> = Vec::new();
    // The Info panel's small map clicked: the Places map at that spot.
    let mut place: Option<(f64, f64)> = None;
    let mut toggle_raw = false;
    // Not while an album picker or a text box has the keys (typing "l" in its search box).
    let typing = pm.pick_busy() || ctx.memory(|m| m.focused().is_some());
    // Keys first, so nothing behind the viewer gets them.
    if !typing {
        ctx.input_mut(|i| {
            use super::photos::Act;
            if i.consume_key(Modifiers::SHIFT, Key::L) {
                acts.push(Act::Remove);
            }
            if i.consume_key(Modifiers::NONE, Key::L) {
                acts.push(Act::AddTo);
            }
            if i.consume_key(Modifiers::NONE, Key::F) {
                acts.push(Act::Favourite);
            }
            if i.consume_key(Modifiers::ALT, Key::R) {
                toggle_raw = true;
            }
            if i.consume_key(Modifiers::SHIFT, Key::R) {
                acts.push(Act::RotateLeft);
            }
            if i.consume_key(Modifiers::NONE, Key::R) {
                acts.push(Act::RotateRight);
            }
            if i.consume_key(Modifiers::NONE, Key::Delete) || i.consume_key(Modifiers::SHIFT, Key::Delete) {
                acts.push(Act::Delete);
            }
            if i.consume_key(Modifiers::COMMAND, Key::Z) {
                pm.out.push(super::photos::Out::Undo);
            }
        });
    }
    ctx.input_mut(|i| {
        if typing {
            return;
        }
        if i.consume_key(Modifiers::NONE, Key::Escape) {
            open = v.escape();
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowRight) || i.consume_key(Modifiers::NONE, Key::PageDown) {
            v.step(1);
            v.next_at = now + v.every;
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowLeft) || i.consume_key(Modifiers::NONE, Key::PageUp) {
            v.step(-1);
            v.next_at = now + v.every;
        }
        if i.consume_key(Modifiers::NONE, Key::Space) {
            if v.playing {
                v.playing = false;
            } else {
                v.play(now);
            }
        }
        if i.consume_key(Modifiers::NONE, Key::Plus) || i.consume_key(Modifiers::NONE, Key::Equals) {
            v.zoom_by(1.5);
        }
        if i.consume_key(Modifiers::NONE, Key::Minus) {
            v.zoom_by(1.0 / 1.5);
        }
        if i.consume_key(Modifiers::NONE, Key::Num0) {
            v.zoom = 1.0;
            v.pan = Vec2::ZERO;
        }
        if i.consume_key(Modifiers::NONE, Key::I) {
            v.info = !v.info;
        }
        if i.consume_key(Modifiers::NONE, Key::E) {
            edit = true;
        }
        if i.consume_key(Modifiers::NONE, Key::Home) {
            v.pos = 0;
        }
        if i.consume_key(Modifiers::NONE, Key::End) {
            v.pos = v.order.len() - 1;
        }
    });
    if v.tick(now) {
        ctx.request_repaint();
    }
    if v.playing {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64((v.next_at - now).max(0.05)));
    }
    let screen = ctx.content_rect();
    let i = v.index();
    if toggle_raw && pm.pics.get(i).is_some_and(|p| !p.stack.is_empty()) {
        v.raw = !v.raw;
    }
    // Ask for this picture, then its neighbours (the slideshow's next first).
    pm.want(ctx, store, i, LARGE);
    let raw_file = pm.pics.get(i).and_then(|p| p.stack.first().cloned());
    if raw_file.is_none() {
        v.raw = false;
    }
    if let (true, Some(f)) = (v.raw, &raw_file) {
        pm.want_id(ctx, store, &f.id, f.v.as_deref(), LARGE);
    }
    let n = v.order.len();
    for d in [1, n.saturating_sub(1)] {
        if n > 1 {
            pm.want(ctx, store, v.order[(v.pos + d) % n], LARGE);
        }
    }
    egui::Area::new(egui::Id::unique("photo_viewer")).order(Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let (rect, resp) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0, Color32::BLACK);
        // Zoom: wheel or pinch, around the middle.
        if resp.hovered() {
            let (scroll, pinch) = ctx.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            if pinch != 1.0 {
                v.zoom_by(pinch);
            } else if scroll != 0.0 {
                v.zoom_by((scroll / 200.0).exp());
            }
        }
        if resp.double_clicked() {
            if v.zoom > 1.0 {
                v.zoom = 1.0;
                v.pan = Vec2::ZERO;
            } else {
                v.zoom = 2.5;
            }
        }
        if resp.dragged() {
            if v.zoom > 1.0 {
                v.pan += resp.drag_delta();
            } else {
                v.drag += resp.drag_delta().x;
            }
        }
        if resp.drag_stopped() {
            let s = swipe(v.drag);
            v.drag = 0.0;
            if s != 0 {
                v.step(s);
                v.next_at = now + v.every;
            }
        }
        // The picture: its own size, fitted, then zoomed and moved.
        let p = &pm.pics[v.index()];
        let (pw, ph) = (p.width.max(1) as f32, p.height.max(1) as f32);
        let (name, date) = (p.name.clone(), date_label(p.taken.as_deref(), p.modified));
        let id = p.id.clone();
        let side = if v.info { INFO_W } else { 0.0 };
        let mut area = rect.shrink2(vec2(0.0, 56.0));
        area.max.x -= side;
        let is_video = p.is_video();
        // Bigger on screen than the 1024 copy: the 2560 screen copy (never the original).
        let want_screen = !is_video && !v.raw && {
            let fit = (area.width() / pw).min(area.height() / ph);
            pw * fit * v.zoom * ctx.pixels_per_point() > LARGE as f32 + 1.0 && pw.max(ph) > LARGE as f32
        };
        if want_screen {
            pm.want(ctx, store, v.index(), SCREEN);
        }
        let want = if want_screen { SCREEN } else { LARGE };
        let best = match (&raw_file, v.raw) {
            (Some(f), true) => pm.best_id(&f.id, LARGE),
            _ if is_video => None,
            _ => pm.best(v.index(), want),
        };
        if is_video {
            // The browser's own player over the picture area; a slideshow would step away from it.
            v.playing = false;
            super::video::show(&id, area, ctx.pixels_per_point());
        } else {
            super::video::hide();
        }
        v.showing = match best.as_ref().map(|b| b.1) {
            Some(ORIGINAL) => "original",
            Some(SCREEN) => "2560",
            Some(LARGE) => "1024",
            Some(SMALL) => "256",
            _ => "none",
        };
        match best {
            Some((t, _)) => {
                let [tw, th] = t.size();
                // The texture's own shape when the listing had no size (not read yet).
                let (w, h) = if p_has_size(pw, ph) { (pw, ph) } else { (tw as f32, th as f32) };
                let fit = (area.width() / w).min(area.height() / h);
                let size = vec2(w, h) * fit * v.zoom;
                // Keep the picture over the middle: no panning past its edges.
                let room = ((size - area.size()) / 2.0).max(Vec2::ZERO);
                v.pan = v.pan.clamp(-room, room);
                let r = Rect::from_center_size(area.center() + v.pan, size);
                painter.image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            None if is_video => {}
            None => {
                painter.text(area.center(), egui::Align2::CENTER_CENTER, "Loading…", egui::FontId::proportional(house::BODY), house::VIEW_BAR_DIM);
            }
        }
        // The bar at the top: close, name and date, where we are; zoom, slideshow, download.
        let bar = Rect::from_min_size(rect.min, vec2(rect.width(), house::VIEW_BAR_HEIGHT));
        painter.rect_filled(bar, 0, house::VIEW_BAR);
        house::target(ctx, "viewer:bar", bar);
        let mut bar_ui =
            ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(house::L, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        house::view_bar_style(&mut bar_ui);
        bar_ui.spacing_mut().item_spacing.x = house::XS;
        if button(&mut bar_ui, "white_close", "Close (Esc)") {
            open = false;
        }
        bar_ui.add_space(house::S);
        bar_ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space(house::M); // two text lines (about 32 px) centred in the 56 px bar
            let shown = match (&raw_file, v.raw) {
                (Some(f), true) => f.name.clone(),
                _ => name.clone(),
            };
            ui.add(egui::Label::new(RichText::new(&shown).size(house::BODY).color(house::VIEW_BAR_TEXT)).truncate());
            ui.label(RichText::new(format!("{date} · {} of {}", v.pos + 1, n)).size(house::NOTE).color(house::VIEW_BAR_DIM));
        });
        bar_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if button(ui, "white_delete", "Move to bin (Delete)") {
                acts.push(super::photos::Act::Delete);
            }
            if pm.open_album().is_some() && button(ui, "white_remove", "Remove from this album (Shift-L)") {
                acts.push(super::photos::Act::Remove);
            }
            if button(ui, "white_photo_library", "Add to album (L)") {
                acts.push(super::photos::Act::AddTo);
            }
            let starred = pm.pics.get(v.index()).is_some_and(|p| p.starred);
            if toggle(ui, if starred { "white_star_fill" } else { "white_star" }, "Favourite (F)", starred) {
                acts.push(super::photos::Act::Favourite);
            }
            if raw_file.is_some() {
                let label = if v.raw { "JPEG" } else { "RAW" };
                let tip = if v.raw { "Show the JPEG (Alt-R)" } else { "Show the RAW file (Alt-R)" };
                if bar_text(ui, None, label, tip, v.raw, "viewer:raw").clicked() {
                    v.raw = !v.raw;
                }
            }
            if button(ui, "white_rotate_right", "Rotate right (R)") {
                acts.push(super::photos::Act::RotateRight);
            }
            if button(ui, "white_rotate_left", "Rotate left (Shift-R)") {
                acts.push(super::photos::Act::RotateLeft);
            }
            if button(ui, "white_download", "Download") {
                if let Some(f) = &raw_file {
                    download(&f.id);
                }
                download(&id);
                crate::house::toast(ctx, "Downloading the original");
            }
            // A small menu under the button: what can really be done with the original.
            let with = bar_text(ui, None, "Open with…", "Open this picture another way", v.menu, "viewer:open-with");
            if with.clicked() {
                v.menu = !v.menu;
            }
            menu_at = Some(with.rect);
            if toggle(ui, "white_info", "Info (I)", v.info) {
                v.info = !v.info;
            }
            if bar_text(ui, Some("edit"), "Edit", "Edit (E)", true, "viewer:edit").clicked() {
                edit = true;
            }
            if toggle(ui, "white_shuffle", if v.shuffle { "Shuffle: on" } else { "Shuffle: off" }, v.shuffle) {
                let seed = (now * 1000.0) as u64;
                v.set_shuffle(!v.shuffle, seed);
            }
            let every = format!("{} s", v.every);
            if bar_text(ui, None, &every, "Slideshow speed", false, "viewer:speed").clicked() {
                let k = EVERY.iter().position(|&e| e == v.every).unwrap_or(1);
                v.every = EVERY[(k + 1) % EVERY.len()];
                v.next_at = now + v.every;
            }
            let (icon, tip) = if v.playing { ("white_pause", "Pause (Space)") } else { ("white_play_arrow", "Slideshow (Space)") };
            if button(ui, icon, tip) {
                if v.playing {
                    v.playing = false;
                } else {
                    v.play(now);
                }
            }
            if button(ui, "white_zoom_in", "Zoom in (+)") {
                v.zoom_by(1.5);
            }
            if button(ui, "white_zoom_out", "Zoom out (−)") {
                v.zoom_by(1.0 / 1.5);
            }
        });
        if v.menu
            && let Some(at) = menu_at
        {
            let mut chosen = None;
            let entries: &[&str] =
                if cfg!(target_arch = "wasm32") { &["Download original", "Edit in Starbase editor"] } else { &["Edit in Starbase editor"] };
            let r = egui::Area::new(egui::Id::unique("photo_open_with"))
                .order(Order::Tooltip)
                .fixed_pos(pos2(at.right() - 200.0, at.bottom() + house::XS))
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_width(200.0);
                        for (k, e) in entries.iter().enumerate() {
                            let row =
                                ui.add(egui::Button::new(RichText::new(*e).size(house::BODY)).frame(false).min_size(vec2(200.0, house::ROW_HEIGHT)));
                            house::target(ctx, &format!("viewer:open-with:{e}"), row.rect);
                            if row.clicked() {
                                chosen = Some(k);
                            }
                        }
                    });
                });
            if let Some(k) = chosen {
                v.menu = false;
                match entries[k] {
                    "Download original" => {
                        download(&id);
                        crate::house::toast(ctx, "Downloading the original");
                    }
                    _ => edit = true,
                }
            } else if ctx.input(|i| i.pointer.any_pressed())
                && !r.response.contains_pointer()
                && !menu_at.is_some_and(|b| ctx.input(|i| i.pointer.interact_pos().is_some_and(|p| b.contains(p))))
            {
                v.menu = false;
            }
        }
        if v.info
            && let Some(p) = pm.pics.get(v.index())
        {
            let in_locked = pm.locked_tab();
            let p = p.clone();
            let people = info_people(pm, ctx, store, &p.id);
            pm.ask_info(ctx, store, &p.id);
            let d = pm.info_detail.as_ref().filter(|f| f.0 == p.id && !f.1.is_null()).map_or(json!({}), |f| f.1.clone());
            let names: Vec<String> = match &people {
                InfoPeople::Found(v, _) => v.iter().map(|(c, _)| c.name.clone()).collect(),
                _ => vec![],
            };
            let albums = pm.alb.albums_of(&p.id);
            let o = info_panel(ui, rect, &p, &albums, in_locked, &people, &d, &names, &mut pm.info_edit);
            drop(albums);
            let (g, lock, shut, who) = (o.go, o.lock, o.close, o.who);
            if let Some((r, lat, lon)) = o.map {
                let clip = Rect::from_min_max(pos2(rect.right() - INFO_W, rect.top() + house::VIEW_BAR_HEIGHT), rect.max);
                let r = r.intersect(clip);
                if r.height() > 4.0 && pm.mini_map(ui, store, r, lat, lon) {
                    place = Some((lat, lon));
                }
            }
            if let Some((f, t)) = o.save {
                pm.save_info(ctx, store, &p.id, info_with(&d, &f, &t));
            }
            goto = g;
            if who.is_some() {
                person = who;
            }
            if shut {
                v.info = false;
            }
            if lock {
                if in_locked {
                    pm.lk.out_req.push(p.id.clone());
                } else {
                    pm.lk.ask = Some(vec![p.id.clone()]);
                }
            }
        }
        // Arrows at the sides (the right one beside the Info panel).
        if n > 1 {
            for (icon, by, x) in [("white_chevron_left", -1, rect.left() + 36.0), ("white_chevron_right", 1, rect.right() - 36.0 - side)] {
                let r = Rect::from_center_size(pos2(x, rect.center().y), vec2(48.0, 48.0));
                let hit = ui.interact(r, egui::Id::unique(("photo_arrow", by)), Sense::click());
                painter.circle_filled(r.center(), 24.0, Color32::from_black_alpha(if hit.hovered() { 180 } else { 110 }));
                if let Some(img) = crate::icons::icon(icon) {
                    img.paint_at(ui, Rect::from_center_size(r.center(), vec2(32.0, 32.0)));
                }
                if hit.clicked() {
                    v.step(by);
                    v.next_at = now + v.every;
                }
            }
        }
    });
    crate::house::show_toast(ctx);
    if open && let Some(id) = pm.pics.get(v.index()).map(|p| p.id.clone()) {
        for act in acts {
            pm.act(ctx, store, act, vec![id.clone()], None);
        }
        // A picture binned or taken out of the album: the next one takes its place.
        v.fit_list(pm.pics.len());
    }
    if open {
        pm.viewer = Some(v);
        if edit {
            v_edit(pm, ctx, store);
        }
    } else {
        super::video::hide();
    }
    if let Some(c) = person {
        pm.viewer = None;
        pm.info_faces = None;
        pm.go(super::photos::albums::Tab::People);
        if let Some(c) = c {
            pm.people_open(ctx, store, c);
        }
        return false;
    }
    if let Some((lat, lon)) = place {
        pm.viewer = None;
        super::video::hide();
        pm.go(super::photos::albums::Tab::Places);
        pm.places_at(lat, lon);
        return false;
    }
    if let Some(c) = goto {
        pm.viewer = None;
        pm.go(super::photos::albums::Tab::Albums);
        pm.open_card(ctx, store, c);
        return false;
    }
    open
}

fn v_edit(pm: &mut PhotoMode, ctx: &egui::Context, store: &Backend) {
    if let Some(v) = pm.viewer.as_mut() {
        v.playing = false;
    }
    super::photo_edit::open(pm, ctx, store);
}

fn p_has_size(w: f32, h: f32) -> bool {
    w > 1.0 && h > 1.0
}

/// The Info panel's sections, top to bottom.
#[derive(Debug, PartialEq)]
pub(super) enum Sec {
    /// Read-only facts: heading, icon, rows (label, value).
    Facts(&'static str, &'static str, Vec<(&'static str, String)>),
    /// A typed field (server name, label, icon, value, placeholder shown when empty).
    Edit(&'static str, &'static str, &'static str, String, String),
    /// The small map at (lat, lon).
    Map(f64, f64),
    People,
    Albums,
}

/// Day of the week of a date (Sakamoto's method), "Sunday".
fn weekday(y: i32, m: u32, d: u32) -> &'static str {
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y - 1 } else { y };
    let n = (y + y / 4 - y / 100 + y / 400 + T[(m.clamp(1, 12) - 1) as usize] + d as i32).rem_euclid(7);
    ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"][n as usize]
}

fn num(v: &Json) -> Option<f64> {
    v.as_f64().filter(|f| f.is_finite())
}

/// "1/12 s", "2 s".
fn shutter(t: f64) -> String {
    if t >= 0.5 { format!("{} s", (t * 10.0).round() / 10.0) } else { format!("1/{} s", (1.0 / t).round()) }
}

/// The Info panel's sections for picture `p` with the server's details `d` (`{}` while asking)
/// and the named people in it (for the default title). Empty fact rows are left out; the typed
/// fields always show (their placeholder when empty).
pub(super) fn info_sections(p: &super::photos::Pic, albums: &[&super::photos::albums::Card], d: &Json, people: &[String]) -> Vec<Sec> {
    let (x, i) = (&d["exif"], &d["info"]);
    let text = |v: &Json| v.as_str().unwrap_or("").to_string();
    let year = p.taken.as_deref().filter(|t| t.len() >= 4).map(|t| t[..4].to_string());
    let default_title = match (people.is_empty(), &year) {
        (false, Some(y)) => format!("{} / {y}", people.join(", ")),
        (false, None) => people.join(", "),
        (true, Some(y)) => format!("Add a title ({y})"),
        _ => "Add a title".into(),
    };
    let mut out = vec![
        Sec::Edit("title", "Title", "edit", text(&i["title"]), default_title),
        Sec::Edit("caption", "Caption", "description", text(&i["caption"]), x["description"].as_str().map_or("Add a caption".into(), str::to_string)),
    ];
    let format = p.name.rsplit_once('.').map(|(_, e)| e.to_uppercase()).unwrap_or_default();
    let mut file = vec![("Name", p.name.clone())];
    if let Some(f) = d["folder"].as_str().filter(|f| !f.is_empty()) {
        file.push(("Folder", f.to_string()));
    }
    if p.width > 0 {
        file.push(("Size", format!("{format}{}{} × {} px, {}", if format.is_empty() { "" } else { ", " }, p.width, p.height, human(p.size))));
    }
    if !p.stack.is_empty() {
        let mut files = vec![format!("{}, {}", p.name, human(p.size))];
        files.extend(p.stack.iter().map(|f| format!("{}, {}", f.name, human(f.size))));
        file.push(("Files", files.join("\n")));
    }
    out.push(Sec::Facts("File", "image", file));
    let mut date = vec![];
    if let Some(t) = p.taken.as_deref().filter(|t| t.len() >= 10)
        && let (Ok(y), Ok(m), Ok(dd)) = (t[..4].parse::<i32>(), t[5..7].parse::<u32>(), t[8..10].parse::<u32>())
    {
        date.push(("Day", weekday(y, m, dd).to_string()));
    }
    date.push(("Taken", date_label(p.taken.as_deref(), p.modified)));
    if let Some(z) = x["zone"].as_str() {
        date.push(("Time zone", format!("UTC{z}")));
    }
    out.push(Sec::Facts("Date", "event", date));
    let mut cam = vec![];
    let camera = [x["make"].as_str(), x["model"].as_str()].into_iter().flatten().collect::<Vec<_>>().join(" ");
    if !camera.is_empty() {
        cam.push(("Camera", camera));
    } else if let Some(c) = &p.camera {
        cam.push(("Camera", c.clone()));
    }
    let exp = [x["iso"].as_u64().map(|n| format!("ISO {n}")), num(&x["exposure"]).map(shutter)].into_iter().flatten().collect::<Vec<_>>();
    if !exp.is_empty() {
        cam.push(("Exposure", exp.join(", ")));
    }
    out.push(Sec::Facts("Camera", "photo_mode", cam));
    let mut lens = vec![];
    if let Some(l) = x["lens"].as_str() {
        lens.push(("Lens", l.to_string()));
    }
    let optics = [
        num(&x["focal"]).map(|f| format!("{} mm", (f * 10.0).round() / 10.0)),
        num(&x["aperture"]).map(|a| format!("ƒ/{}", (a * 10.0).round() / 10.0)),
        x["focal35"].as_u64().map(|f| format!("{f} mm (35 mm equiv.)")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    if !optics.is_empty() {
        lens.push(("Optics", optics.join(", ")));
    }
    out.push(Sec::Facts("Lens", "zoom_in", lens));
    if let Some((lat, lon)) = p.gps {
        let mut place = vec![];
        if let Some(pl) = &p.place {
            place.push(("Place", pl.clone()));
        }
        place.push(("Position", format!("{lat:.4}, {lon:.4}")));
        if let Some(a) = num(&x["alt"]) {
            place.push(("Altitude", format!("{a:.0} m")));
        }
        out.push(Sec::Facts("Place", "location_on", place));
        out.push(Sec::Map(lat, lon));
    }
    out.push(Sec::People);
    if !albums.is_empty() {
        out.push(Sec::Albums);
    }
    let labels = i["labels"].as_array().map(|a| a.iter().filter_map(|l| l.as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default();
    out.push(Sec::Edit("labels", "Labels", "label", labels, "Add labels, separated by commas".into()));
    for (k, label, icon, exif) in [
        ("subject", "Subject", "sticky_note_2", None),
        ("copyright", "Copyright", "description", x["copyright"].as_str()),
        ("artist", "Artist", "group", x["artist"].as_str()),
        ("licence", "Licence", "language", None),
    ] {
        out.push(Sec::Edit(k, label, icon, text(&i[k]), exif.map_or_else(|| format!("Add {}", label.to_lowercase()), str::to_string)));
    }
    let mut keys: Vec<String> = p.place.iter().flat_map(|pl| pl.split(',').map(|s| s.trim().to_string())).collect();
    for k in d["keywords"].as_array().into_iter().flatten().filter_map(|k| k.as_str()) {
        if !keys.iter().any(|q| q.eq_ignore_ascii_case(k)) {
            keys.push(k.to_string());
        }
    }
    keys.retain(|k| !k.is_empty());
    if !keys.is_empty() {
        out.push(Sec::Facts("Keywords", "search", vec![("Keywords", keys.join(" · "))]));
    }
    if let Some(u) = x["unique_id"].as_str() {
        out.push(Sec::Facts("Photo ID", "numbers_123", vec![("Camera ID", u.to_string())]));
    }
    out.retain(|s| !matches!(s, Sec::Facts(_, _, rows) if rows.is_empty()));
    out
}

/// The same facts as one list (the test hook's `info_rows`): fact rows, then typed fields that are set.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(super) fn info_lines(p: &super::photos::Pic, albums: &[&super::photos::albums::Card], d: &Json) -> Vec<(&'static str, String)> {
    info_sections(p, albums, d, &[])
        .into_iter()
        .flat_map(|s| match s {
            Sec::Facts(_, _, rows) => rows,
            Sec::Edit(_, label, _, v, _) if !v.is_empty() => vec![(label, v)],
            _ => vec![],
        })
        .collect()
}

/// The typed fields with `field` set to `text` (labels split at commas), as the server takes them.
fn info_with(d: &Json, field: &str, text: &str) -> Json {
    let mut i = if d["info"].is_object() { d["info"].clone() } else { json!({}) };
    i[field] =
        if field == "labels" { json!(text.split(',').map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>()) } else { json!(text.trim()) };
    i
}

fn human(b: u64) -> String {
    match b {
        0..1_000 => format!("{b} B"),
        1_000..1_000_000 => format!("{:.0} KB", b as f64 / 1e3),
        _ => format!("{:.1} MB", b as f64 / 1e6),
    }
}

/// The Info panel's "People" section, asked of the server once per picture shown.
enum InfoPeople {
    Asking,
    Off,
    /// Named people (card for their page, their face picture if here yet) and how many unnamed.
    Found(Vec<(super::photos::albums::Card, Option<egui::TextureHandle>)>, u64),
}

/// The faces in picture `id` for the Info panel: asks the server (once), then the face pictures.
fn info_people(pm: &mut PhotoMode, ctx: &egui::Context, store: &Backend, id: &str) -> InfoPeople {
    pm.ask_faces_in(ctx, store, id);
    let v = pm.info_faces.as_ref().map_or(Json::Null, |f| f.1.clone());
    if v.is_null() {
        return InfoPeople::Asking;
    }
    let people: Vec<(super::photos::albums::Card, Option<egui::TextureHandle>)> = v["people"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|j| {
            let key = format!("face/{}/{}", j["face"], j["pic"].as_str().unwrap_or(""));
            pm.want_id(ctx, store, &key, j["v"].as_str(), SMALL);
            let t = pm.tex.get(&(key, SMALL), pm.frame);
            let c = super::photos::albums::Card {
                id: j["id"].as_str().unwrap_or("").to_string(),
                name: j["name"].as_str().unwrap_or("").to_string(),
                ..Default::default()
            };
            (c, t)
        })
        .collect();
    let unnamed = v["unnamed"].as_u64().unwrap_or(0);
    if v["on"] == json!(false) && people.is_empty() && unnamed == 0 {
        return InfoPeople::Off;
    }
    InfoPeople::Found(people, unnamed)
}

/// The quiet line under "People" when there is nothing to link to.
fn people_note(p: &InfoPeople) -> Option<&'static str> {
    match p {
        InfoPeople::Asking => Some("Looking for faces…"),
        InfoPeople::Off => Some("Face recognition is off."),
        InfoPeople::Found(v, 0) if v.is_empty() => Some("No faces found"),
        InfoPeople::Found(..) => None,
    }
}

/// "1 unnamed face", "3 unnamed faces".
fn unnamed_label(n: u64) -> String {
    format!("{n} unnamed face{}", if n == 1 { "" } else { "s" })
}

/// The faces' size in the Info panel's People section.
const FACE_D: f32 = 28.0;

/// How wide the labels' column is in the Info panel.
const INFO_LABEL_W: f32 = 88.0;

/// One fact row in the Info panel: dim label, then the value (album names as links).
fn info_row(
    c: &mut egui::Ui,
    ctx: &egui::Context,
    k: &str,
    v: &str,
    albums: &[&super::photos::albums::Card],
    go: &mut Option<super::photos::albums::Card>,
) {
    c.horizontal_top(|row| {
        row.spacing_mut().item_spacing.x = 0.0;
        let (lab, _) = row.allocate_exact_size(vec2(INFO_LABEL_W, house::BODY + house::XS), Sense::hover());
        row.painter().text(
            pos2(lab.left(), lab.center().y),
            egui::Align2::LEFT_CENTER,
            k,
            egui::FontId::proportional(house::NOTE),
            house::VIEW_BAR_DIM,
        );
        row.vertical(|val| {
            val.set_width(val.available_width());
            val.spacing_mut().item_spacing.y = house::XS;
            if k == "Albums" {
                for (n, a) in albums.iter().enumerate() {
                    let r = val.add(egui::Label::new(RichText::new(&a.name).size(house::BODY).color(house::VIEW_BAR_LINK)).sense(Sense::click()));
                    house::target(ctx, &format!("viewer:info:album:{n}"), r.rect);
                    let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
                    if r.hovered() {
                        val.painter().hline(r.rect.x_range(), r.rect.bottom(), egui::Stroke::new(1.0, house::VIEW_BAR_LINK));
                    }
                    if r.clicked() {
                        *go = Some((*a).clone());
                    }
                }
            } else {
                val.add(egui::Label::new(RichText::new(v).size(house::BODY).color(house::VIEW_BAR_TEXT)).wrap());
            }
        });
    });
}

/// What was asked for in the Info panel this frame.
struct InfoOut {
    /// An album clicked.
    go: Option<super::photos::albums::Card>,
    /// "Move to Locked folder" (inside Locked, "Move out").
    lock: bool,
    close: bool,
    /// A person clicked (Some(None): the unnamed faces).
    who: Option<Option<super::photos::albums::Card>>,
    /// A typed field finished: (field, text).
    save: Option<(String, String)>,
    /// Where the small map goes: (screen rect, lat, lon).
    map: Option<(Rect, f64, f64)>,
}

/// A section heading in the Info panel: its icon, then the name in capitals.
fn info_heading(c: &mut egui::Ui, heading: &str, icon: &str) {
    c.add_space(house::L);
    c.horizontal(|h| {
        h.spacing_mut().item_spacing.x = house::S;
        if let Some(img) = crate::icons::icon_in(icon, house::VIEW_BAR_DIM) {
            h.add(img.fit_to_exact_size(vec2(house::BODY + 2.0, house::BODY + 2.0)));
        }
        h.label(RichText::new(heading.to_uppercase()).size(house::CAPS).strong().color(house::VIEW_BAR_DIM));
    });
    c.add_space(house::M);
}

/// The Info panel: a title row with a close button, then the groups of [`info_groups`] as
/// two-column rows (dim label, value), then the Locked folder button. Returns the album whose name
/// was clicked, whether "Move to Locked folder" (inside Locked, "Move out") was clicked, and
/// whether the panel's close button was.
#[allow(clippy::too_many_arguments)]
fn info_panel(
    ui: &mut egui::Ui,
    rect: Rect,
    p: &super::photos::Pic,
    albums: &[&super::photos::albums::Card],
    in_locked: bool,
    people: &InfoPeople,
    d: &Json,
    names: &[String],
    edit: &mut Option<(String, String)>,
) -> InfoOut {
    let writable = d["writable"].as_bool() == Some(true);
    let mut save = None;
    let mut map = None;
    let ctx = ui.ctx().clone();
    let panel = Rect::from_min_max(pos2(rect.right() - INFO_W, rect.top() + house::VIEW_BAR_HEIGHT), pos2(rect.right(), rect.bottom()));
    ui.painter().rect_filled(panel, 0, house::VIEW_BAR);
    ui.painter().vline(panel.left(), panel.y_range(), egui::Stroke::new(1.0, house::VIEW_BAR_FIELD));
    house::target(&ctx, "viewer:info-panel", panel);
    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(panel.shrink(house::L)).layout(egui::Layout::top_down(egui::Align::Min)));
    house::view_bar_style(&mut c);
    c.spacing_mut().item_spacing = vec2(house::S, 0.0);
    let mut close = false;
    c.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.add_space(house::S); // centre the title on the 36 px close button
            ui.label(RichText::new("Info").size(house::BODY).strong().color(house::VIEW_BAR_TEXT));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let r = house::icon_button(ui, "white_close", "Close Info (I)", false);
            house::target(&ctx, "viewer:info:close", r.rect);
            close = r.clicked();
        });
    });
    let mut go = None;
    let mut who = None;
    let mut lock = false;
    egui::ScrollArea::vertical().auto_shrink([false, true]).show(&mut c, |c| {
        for sec in info_sections(p, albums, d, names) {
            let (heading, icon, rows) = match sec {
                Sec::Facts(h, i, rows) => (h, i, rows),
                Sec::Albums => ("Albums", "photo_library", vec![("Albums", String::new())]),
                Sec::Map(lat, lon) => {
                    c.add_space(house::M);
                    let (r, _) = c.allocate_exact_size(vec2(c.available_width(), 120.0), Sense::hover());
                    map = Some((r, lat, lon));
                    continue;
                }
                Sec::Edit(field, label, icon, value, ph) => {
                    info_heading(c, label, icon);
                    let id = egui::Id::unique(("info_field", field));
                    if let Some(e) = edit.as_mut().filter(|e| e.0 == field) {
                        let te = if field == "caption" {
                            egui::TextEdit::multiline(&mut e.1).desired_rows(2)
                        } else {
                            egui::TextEdit::singleline(&mut e.1)
                        };
                        let r = c.add(te.id(id).desired_width(f32::INFINITY).font(egui::FontId::proportional(house::BODY)).hint_text(ph.as_str()));
                        house::target(&ctx, &format!("viewer:info:field:{field}"), r.rect);
                        if r.lost_focus() {
                            if !ctx.input(|i| i.key_pressed(egui::Key::Escape)) && e.1.trim() != value.trim() {
                                save = Some((field.to_string(), e.1.clone()));
                            }
                            *edit = None;
                        } else if !r.has_focus() {
                            r.request_focus();
                        }
                    } else {
                        let (shown, colour) = if value.is_empty() { (ph, house::VIEW_BAR_DIM) } else { (value.clone(), house::VIEW_BAR_TEXT) };
                        let sense = if writable { Sense::click() } else { Sense::hover() };
                        let r = c.add(egui::Label::new(RichText::new(shown).size(house::BODY).color(colour)).wrap().sense(sense));
                        house::target(&ctx, &format!("viewer:info:edit:{field}"), r.rect);
                        if writable {
                            let r = r.on_hover_cursor(egui::CursorIcon::Text).on_hover_text(format!("Edit {}", label.to_lowercase()));
                            if r.clicked() {
                                *edit = Some((field.to_string(), value));
                            }
                        }
                    }
                    continue;
                }
                Sec::People => {
                    // PEOPLE: each named person (round face + name, opens their page), then the unnamed faces.
                    c.add_space(house::L);
                    let top = c.cursor().top();
                    c.label(RichText::new("PEOPLE").size(house::CAPS).strong().color(house::VIEW_BAR_DIM));
                    c.add_space(house::M);
                    c.spacing_mut().item_spacing.y = house::XS;
                    if let Some(note) = people_note(people) {
                        c.label(RichText::new(note).size(house::NOTE).color(house::VIEW_BAR_DIM));
                    }
                    if let InfoPeople::Found(named, unnamed) = people {
                        for (n, (card, tex)) in named.iter().enumerate() {
                            let r = c.horizontal(|row| {
                                row.spacing_mut().item_spacing.x = house::S;
                                let (face, _) = row.allocate_exact_size(vec2(FACE_D, FACE_D), Sense::hover());
                                match tex {
                                    Some(t) => {
                                        egui::Image::new((t.id(), face.size())).corner_radius(FACE_D / 2.0).paint_at(row, face);
                                    }
                                    None => {
                                        row.painter().circle_filled(face.center(), FACE_D / 2.0, house::VIEW_BAR_FIELD);
                                    }
                                }
                                row.label(RichText::new(&card.name).size(house::BODY).color(house::VIEW_BAR_LINK));
                            });
                            let r = c
                                .interact(r.response.rect, egui::Id::unique(("info_person", n)), Sense::click())
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            house::target(&ctx, &format!("photos:info-person:{n}"), r.rect);
                            if r.hovered() {
                                c.painter().hline(r.rect.x_range(), r.rect.bottom(), egui::Stroke::new(1.0, house::VIEW_BAR_LINK));
                            }
                            if r.clicked() {
                                who = Some(Some(card.clone()));
                            }
                        }
                        if *unnamed > 0 {
                            let r = c
                                .add(
                                    egui::Label::new(RichText::new(unnamed_label(*unnamed)).size(house::BODY).color(house::VIEW_BAR_LINK))
                                        .sense(Sense::click()),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            house::target(&ctx, "photos:info-people-unnamed", r.rect);
                            if r.clicked() {
                                who = Some(None);
                            }
                        }
                    }
                    house::target(
                        &ctx,
                        "photos:info-people",
                        Rect::from_min_max(pos2(c.max_rect().left(), top), pos2(c.max_rect().right(), c.cursor().top())),
                    );
                    c.spacing_mut().item_spacing.y = 0.0;
                    continue;
                }
            };
            info_heading(c, heading, icon);
            for (i, (k, v)) in rows.iter().enumerate() {
                if i > 0 {
                    c.add_space(house::S);
                }
                info_row(c, &ctx, k, v, albums, &mut go);
            }
        }
        c.add_space(house::L);
        c.painter().hline(c.max_rect().x_range(), c.cursor().top(), egui::Stroke::new(1.0, house::VIEW_BAR_FIELD));
        c.add_space(house::L);
        let label = if in_locked { "Move out of Locked folder" } else { "Move to Locked folder" };
        let r = c.add(house::secondary_dark(label));
        house::target(&ctx, "viewer:info:lock", r.rect);
        lock = r.clicked();
    });
    InfoOut { go, lock, close, who, save, map }
}

/// A white icon button on the dark bar (its hook target is `viewer:<icon>`).
fn button(ui: &mut egui::Ui, icon: &str, tip: &str) -> bool {
    toggle(ui, icon, tip, false)
}

/// A white icon button on the dark bar, lit when `on`.
fn toggle(ui: &mut egui::Ui, icon: &str, tip: &str, on: bool) -> bool {
    let r = house::icon_button(ui, icon, tip, on);
    house::target(ui.ctx(), &format!("viewer:{}", icon.trim_start_matches("white_")), r.rect);
    r.clicked()
}

/// A text button on the dark bar (Open with…, the slideshow speed, RAW / JPEG, Edit), with an icon
/// in front when one is named; lit when `on`.
fn bar_text(ui: &mut egui::Ui, icon: Option<&str>, label: &str, tip: &str, on: bool, target: &str) -> egui::Response {
    let text = RichText::new(label.to_string()).size(house::BODY).color(house::VIEW_BAR_TEXT);
    let b = match icon.and_then(|i| crate::icons::icon_in(i, house::VIEW_BAR_TEXT)) {
        Some(img) => egui::Button::image_and_text(img.fit_to_exact_size(vec2(house::ICON - house::XS, house::ICON - house::XS)), text),
        None => egui::Button::new(text),
    };
    let b = b.selected(on).frame_when_inactive(on).corner_radius(egui::CornerRadius::same(house::BUTTON_RADIUS));
    let r = ui.add(b.min_size(vec2(house::BUTTON_HEIGHT, house::BUTTON_HEIGHT))).on_hover_text(tip);
    house::target(ui.ctx(), target, r.rect);
    r
}

/// Downloads the original (the server sends it as an attachment).
fn download(id: &str) {
    #[cfg(target_arch = "wasm32")]
    if let Some(w) = web_sys::window() {
        let _ = w.location().set_href(&format!("api/files/{id}"));
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = id;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_sections_in_order_with_exif_and_typed_fields() {
        let mut p = super::super::photos::Pic { name: "IMG_2041.jpg".into(), ..Default::default() };
        let heads = |s: &[Sec]| {
            s.iter()
                .map(|x| match x {
                    Sec::Facts(h, ..) => *h,
                    Sec::Edit(_, l, ..) => *l,
                    Sec::Map(..) => "map",
                    Sec::People => "People",
                    Sec::Albums => "Albums",
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            heads(&info_sections(&p, &[], &json!({}), &[])),
            ["Title", "Caption", "File", "Date", "People", "Labels", "Subject", "Copyright", "Artist", "Licence"]
        );
        p.width = 4032;
        p.height = 3024;
        p.size = 3_100_000;
        p.taken = Some("2024-07-14T19:05:33".into());
        p.gps = Some((38.72, -9.14));
        p.place = Some("Lisbon, Portugal".into());
        let d = json!({"folder": "ann/Trip", "exif": {"make": "Google", "model": "Pixel 8", "iso": 640, "exposure": 0.0833, "lens": "Main", "focal": 6.9, "aperture": 1.8, "focal35": 26, "zone": "+01:00", "alt": 12.4, "unique_id": "U1"}, "info": {"title": "Bay", "labels": ["sea"]}, "keywords": ["Trip", "sea"]});
        let s = info_sections(&p, &[], &d, &["Ann".into()]);
        assert_eq!(
            heads(&s),
            [
                "Title",
                "Caption",
                "File",
                "Date",
                "Camera",
                "Lens",
                "Place",
                "map",
                "People",
                "Labels",
                "Subject",
                "Copyright",
                "Artist",
                "Licence",
                "Keywords",
                "Photo ID"
            ]
        );
        let rows = info_lines(&p, &[], &d);
        let get = |k: &str| rows.iter().find(|r| r.0 == k).map(|r| r.1.clone()).unwrap_or_default();
        assert_eq!((get("Title"), get("Day"), get("Time zone")), ("Bay".into(), "Sunday".into(), "UTC+01:00".into()));
        assert_eq!((get("Exposure"), get("Optics")), ("ISO 640, 1/12 s".into(), "6.9 mm, ƒ/1.8, 26 mm (35 mm equiv.)".into()));
        assert_eq!((get("Position"), get("Altitude"), get("Size")), ("38.7200, -9.1400".into(), "12 m".into(), "JPG, 4032 × 3024 px, 3.1 MB".into()));
        assert_eq!(get("Keywords"), "Lisbon · Portugal · Trip · sea");
        assert!(matches!(&s[0], Sec::Edit("title", _, _, v, _) if v == "Bay"));
        assert_eq!(info_with(&d, "labels", "sea, , boats ")["labels"], json!(["sea", "boats"]));
    }

    #[test]
    fn info_people_says_what_it_found() {
        assert_eq!(people_note(&InfoPeople::Asking), Some("Looking for faces…"));
        assert_eq!(people_note(&InfoPeople::Off), Some("Face recognition is off."));
        assert_eq!(people_note(&InfoPeople::Found(Vec::new(), 0)), Some("No faces found"));
        assert_eq!(people_note(&InfoPeople::Found(Vec::new(), 2)), None);
        assert_eq!(unnamed_label(1), "1 unnamed face");
        assert_eq!(unnamed_label(3), "3 unnamed faces");
    }

    #[test]
    fn steps_go_round() {
        let mut v = Viewer::new(0, 3);
        v.step(-1);
        assert_eq!(v.index(), 2);
        v.step(1);
        assert_eq!(v.index(), 0);
        v.zoom = 3.0;
        v.step(1);
        assert_eq!((v.index(), v.zoom), (1, 1.0), "a new picture starts fitted");
        let mut one = Viewer::new(5, 1);
        one.step(1);
        assert_eq!(one.index(), 0);
    }

    #[test]
    fn the_slideshow_keeps_time_and_shuffles() {
        let mut v = Viewer::new(2, 10);
        v.every = 5.0;
        v.play(100.0);
        assert!(!v.tick(104.0));
        assert!(v.tick(105.0));
        assert_eq!(v.index(), 3);
        v.set_shuffle(true, 42);
        assert_eq!(v.index(), 3, "shuffling keeps the picture shown");
        let mut seen: Vec<usize> = v.order.clone();
        seen.sort();
        assert_eq!(seen, (0..10).collect::<Vec<_>>(), "every picture once");
        assert_ne!(v.order, (0..10).collect::<Vec<_>>());
        assert_eq!(order(5, false, 3, 9), vec![0, 1, 2, 3, 4]);
        v.set_shuffle(false, 0);
        assert_eq!(v.index(), 3);
    }

    #[test]
    fn escape_peels_one_layer_at_a_time() {
        // (menu, info) before → (menu, info) after, still open?
        let table = [
            ((false, false), (false, false), false),
            ((false, true), (false, false), true),
            ((true, false), (false, false), true),
            ((true, true), (false, true), true),
        ];
        for ((menu, info), after, open) in table {
            let mut v = Viewer::new(0, 3);
            v.menu = menu;
            v.info = info;
            assert_eq!(v.escape(), open, "menu={menu} info={info}");
            assert_eq!((v.menu, v.info), after, "menu={menu} info={info}");
        }
        // Info open: the first Escape closes only Info, the second closes the viewer.
        let mut v = Viewer::new(0, 3);
        v.info = true;
        assert!(v.escape());
        assert!(!v.escape());
    }

    #[test]
    fn swipes_need_some_distance() {
        assert_eq!(swipe(-100.0), 1, "to the left: the next one");
        assert_eq!(swipe(100.0), -1);
        assert_eq!(swipe(20.0), 0);
    }

    #[test]
    fn a_shorter_list_keeps_the_viewer_inside_it() {
        let mut v = Viewer::new(9, 10);
        v.fit_list(4);
        assert_eq!((v.index(), v.order.len()), (3, 4));
    }
}
