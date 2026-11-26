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

use super::photos::{LARGE, ORIGINAL, PhotoMode, SMALL, date_label};
use crate::store::Backend;

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
const INFO_W: f32 = 300.0;

pub(super) fn ui(pm: &mut PhotoMode, ctx: &egui::Context, store: &Backend) -> bool {
    if pm.viewer.is_none() || pm.pics.is_empty() {
        pm.viewer = None;
        pm.editor = None;
        return false;
    }
    if pm.editor.is_some() {
        super::photo_edit::ui(pm, ctx, store);
        return true;
    }
    let Some(mut v) = pm.viewer.take() else { return false };
    v.fit_list(pm.pics.len());
    let now = ctx.input(|i| i.time);
    let mut open = true;
    let mut edit = false;
    let mut menu_at: Option<Rect> = None;
    // What the person asked for on this picture (keys or buttons): L add to album, Shift-L out of
    // the open album, F favourite, Delete move to bin. Run after drawing, on the picture shown.
    let mut acts: Vec<super::photos::Act> = Vec::new();
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
    // Ask for this picture, then its neighbours (the slideshow's next first).
    pm.want(ctx, store, i, LARGE);
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
        let want_original = {
            let fit = (area.width() / pw).min(area.height() / ph);
            pw * fit * v.zoom * ctx.pixels_per_point() > LARGE as f32 + 1.0 && pw.max(ph) > LARGE as f32
        };
        if want_original {
            pm.want(ctx, store, v.index(), ORIGINAL);
        }
        let best = pm.best(v.index(), if want_original { ORIGINAL } else { LARGE });
        v.showing = match best.as_ref().map(|b| b.1) {
            Some(ORIGINAL) => "original",
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
            None => {
                painter.text(area.center(), egui::Align2::CENTER_CENTER, "Loading…", egui::FontId::proportional(16.0), Color32::from_gray(200));
            }
        }
        // The bar at the top: close, name and date, where we are; zoom, slideshow, download.
        let bar = Rect::from_min_size(rect.min, vec2(rect.width(), 56.0));
        painter.rect_filled(bar, 0, Color32::from_black_alpha(160));
        let mut bar_ui =
            ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(12.0, 8.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        if button(&mut bar_ui, "white_close", "Close (Esc)") {
            open = false;
        }
        bar_ui.add_space(8.0);
        bar_ui.vertical(|ui| {
            ui.label(RichText::new(&name).size(15.0).color(Color32::WHITE));
            ui.label(RichText::new(format!("{date} · {} of {}", v.pos + 1, n)).size(12.0).color(Color32::from_gray(200)));
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
            if button(ui, "white_download", "Download") {
                download(&id);
                crate::house::toast(ctx, "Downloading the original");
            }
            // A small menu under the button: what can really be done with the original.
            let with = ui
                .add(egui::Button::new(RichText::new("Open with…").size(13.0).color(Color32::WHITE)).frame(false).min_size(vec2(40.0, 36.0)))
                .on_hover_text("Open this picture another way");
            if with.clicked() {
                v.menu = !v.menu;
            }
            menu_at = Some(with.rect);
            if toggle(ui, "white_info", "Info (I)", v.info) {
                v.info = !v.info;
            }
            let edit_icon = crate::icons::icon_in("edit", Color32::WHITE).map(|i| i.fit_to_exact_size(vec2(20.0, 20.0)));
            let b = match edit_icon {
                Some(img) => egui::Button::image_and_text(img, RichText::new("Edit").size(14.0).color(Color32::WHITE)),
                None => egui::Button::new(RichText::new("Edit").size(14.0).color(Color32::WHITE)),
            };
            if ui.add(b.fill(Color32::from_white_alpha(40)).corner_radius(18).min_size(vec2(0.0, 36.0))).on_hover_text("Edit (E)").clicked() {
                edit = true;
            }
            if toggle(ui, "white_shuffle", if v.shuffle { "Shuffle: on" } else { "Shuffle: off" }, v.shuffle) {
                let seed = (now * 1000.0) as u64;
                v.set_shuffle(!v.shuffle, seed);
            }
            let every = format!("{} s", v.every);
            if ui
                .add(egui::Button::new(RichText::new(every).size(13.0).color(Color32::WHITE)).frame(false).min_size(vec2(40.0, 36.0)))
                .on_hover_text("Slideshow speed")
                .clicked()
            {
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
                .fixed_pos(pos2(at.right() - 200.0, at.bottom() + 6.0))
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_width(200.0);
                        for (k, e) in entries.iter().enumerate() {
                            if ui.add(egui::Button::new(*e).frame(false).min_size(vec2(200.0, 32.0))).clicked() {
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
            info_panel(ui, rect, p);
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

/// The facts about a picture. Where it was taken only when the server sent it (people who may edit).
pub(super) fn info_lines(p: &super::photos::Pic) -> Vec<(&'static str, String)> {
    let mut rows = vec![("Date taken", super::photos::date_label(p.taken.as_deref(), p.modified))];
    if let Some(c) = &p.camera {
        rows.push(("Camera", c.clone()));
    }
    if p.width > 0 {
        rows.push(("Size", format!("{} × {} px, {}", p.width, p.height, human(p.size))));
    }
    if let Some((lat, lon)) = p.gps {
        rows.push(("Location", format!("{lat:.5}, {lon:.5}")));
    }
    rows
}

fn human(b: u64) -> String {
    match b {
        0..1_000 => format!("{b} B"),
        1_000..1_000_000 => format!("{:.0} KB", b as f64 / 1e3),
        _ => format!("{:.1} MB", b as f64 / 1e6),
    }
}

fn info_panel(ui: &mut egui::Ui, rect: Rect, p: &super::photos::Pic) {
    let panel = Rect::from_min_max(pos2(rect.right() - INFO_W, rect.top() + 56.0), pos2(rect.right(), rect.bottom()));
    ui.painter().rect_filled(panel, 0, Color32::from_black_alpha(190));
    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(panel.shrink(16.0)));
    c.label(RichText::new("Info").size(16.0).strong().color(Color32::WHITE));
    c.add_space(8.0);
    for (k, v) in info_lines(p) {
        c.label(RichText::new(k).size(12.0).color(Color32::from_gray(170)));
        c.label(RichText::new(v).size(14.0).color(Color32::WHITE));
        c.add_space(6.0);
    }
}

/// A white icon button on the dark bar.
fn button(ui: &mut egui::Ui, icon: &str, tip: &str) -> bool {
    toggle(ui, icon, tip, false)
}

/// A white icon button on the dark bar, lit when `on`.
fn toggle(ui: &mut egui::Ui, icon: &str, tip: &str, on: bool) -> bool {
    let b = match crate::icons::icon(icon) {
        Some(img) => egui::Button::image(img.fit_to_exact_size(vec2(24.0, 24.0))),
        None => egui::Button::new(RichText::new(tip).color(Color32::WHITE)),
    };
    let b = if on { b.fill(Color32::from_white_alpha(60)).corner_radius(20) } else { b.frame(false) };
    ui.add(b.min_size(vec2(40.0, 40.0))).on_hover_text(tip).clicked()
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
