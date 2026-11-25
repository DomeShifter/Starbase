//! Drive's file preview: a dark overlay over the file list, as in Google Drive, for every file
//! that doesn't open in an editor (pictures, PDFs, films, sound, text, anything else).
//!
//! - Opened by a double click, Enter, or right-click › Preview. Esc or ✕ closes; the arrows (keys
//!   or the buttons at the sides) go through the other such files of the folder or view shown.
//! - Top bar: the file's icon and name; Open with (a new tab), Download, Details (a panel at the
//!   right) and close.
//! - Pictures are drawn here (decoded as Photos does, EXIF orientation applied): fit to the
//!   window; Ctrl+wheel, + and − or the buttons zoom, 0 fits, a drag pans.
//! - Text: `textpreview::show`.
//! - In the browser a PDF, a film or sound plays in a page element laid exactly over the content
//!   area (`api/files/ID/raw`: the server's types, CSP and sandbox), removed when the preview
//!   closes or moves to another file. The desktop app offers those as a download.
//! - Anything else: "No preview available", with Download and Open with.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use std::sync::{Arc, Mutex};

use egui::{Color32, Key, Modifiers, Order, Rect, RichText, Sense, Vec2, pos2, vec2};

use super::textpreview;
use crate::store::{Backend, Method};
#[cfg(target_arch = "wasm32")]
use std::sync::atomic::{AtomicBool, Ordering};

/// Text files fetched, at most; only the first [`textpreview::MAX_BYTES`] are kept and shown,
/// with a note saying so.
pub(super) const TEXT_MAX: u64 = 16 * 1024 * 1024;
/// Pictures read whole into memory, at most.
pub(super) const IMAGE_MAX: u64 = 64 * 1024 * 1024;
const MAX_ZOOM: f32 = 8.0;
const BAR: f32 = 64.0;
const INFO_W: f32 = 320.0;

/// How a file is shown in the preview.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Shown {
    Image,
    Pdf,
    Video,
    Audio,
    Text,
    /// Spreadsheets and documents open in their editor; everything else has no preview.
    None,
}

impl Shown {
    pub(super) fn label(self) -> &'static str {
        match self {
            Shown::Image => "image",
            Shown::Pdf => "pdf",
            Shown::Video => "video",
            Shown::Audio => "audio",
            Shown::Text => "text",
            Shown::None => "none",
        }
    }

    /// Whether this build can show it (the desktop app has no page elements for PDF and media).
    pub(super) fn here(self) -> Shown {
        if cfg!(target_arch = "wasm32") || matches!(self, Shown::Image | Shown::Text) { self } else { Shown::None }
    }
}

pub(super) fn shown_as(name: &str) -> Shown {
    match super::items::file_kind(name) {
        "image" => Shown::Image,
        "pdf" => Shown::Pdf,
        "video" => Shown::Video,
        "audio" => Shown::Audio,
        "text" => Shown::Text,
        // CSV, Markdown variants, HTML/SVG/XML source and other code: shown as text (never run).
        _ if textpreview::handles(name) => Shown::Text,
        _ => Shown::None,
    }
}

/// Whether a key pressed on the page (`KeyboardEvent.key` and its modifiers) closes the preview:
/// Esc on its own. Used by the page-level listener that still hears Esc once a PDF frame or a
/// film has the keyboard focus (egui only hears keys while the canvas has it).
pub(super) fn closes_preview(key: &str, ctrl: bool, alt: bool, meta: bool, shift: bool) -> bool {
    matches!(key, "Escape" | "Esc") && !(ctrl || alt || meta || shift)
}

/// The part of a fetched text file that is kept: a byte over [`textpreview::MAX_BYTES`] so the
/// preview knows to say it shows only the first part.
fn keep_head(mut bytes: Vec<u8>) -> Vec<u8> {
    bytes.truncate(textpreview::MAX_BYTES + 1);
    bytes.shrink_to_fit();
    bytes
}

enum Got {
    Image(Option<egui::ColorImage>),
    Bytes(Vec<u8>),
    Failed(String),
}

enum Content {
    Loading,
    Image(egui::TextureHandle),
    Text(Vec<u8>),
    /// Shown by a page element (PDF, film, sound).
    Element,
    /// No preview: why (shown under "No preview available").
    Nothing(String),
}

/// What the preview asks Drive to do.
pub(super) enum Act {
    Close,
    Step(i32),
    Download(String, String),
    NewTab(String),
}

pub(super) struct FilePreview {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub modified: i64,
    pub owner: String,
    pub shown: Shown,
    pub info: bool,
    pub zoom: f32,
    pub pan: Vec2,
    /// Where we are in the files the arrows go through, and how many there are.
    pub at: (usize, usize),
    content: Content,
    got: Arc<Mutex<Vec<(String, Got)>>>,
    #[cfg(target_arch = "wasm32")]
    element: Option<Element>,
}

impl FilePreview {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn open(
        ctx: &egui::Context,
        store: &Backend,
        id: &str,
        name: &str,
        size: u64,
        modified: i64,
        owner: String,
        at: (usize, usize),
    ) -> Self {
        let shown = shown_as(name).here();
        let mut p = FilePreview {
            id: id.into(),
            name: name.into(),
            size,
            modified,
            owner,
            shown,
            info: false,
            zoom: 1.0,
            pan: Vec2::ZERO,
            at,
            content: Content::Loading,
            got: Arc::default(),
            #[cfg(target_arch = "wasm32")]
            element: None,
        };
        p.content = match shown {
            Shown::Image if size > IMAGE_MAX => Content::Nothing("This picture is too big to show here.".into()),
            Shown::Text if size > TEXT_MAX => Content::Nothing("This file is too big to show here.".into()),
            Shown::Image | Shown::Text => {
                p.fetch(ctx, store);
                Content::Loading
            }
            Shown::Pdf | Shown::Video | Shown::Audio => Content::Element,
            Shown::None => Content::Nothing(String::new()),
        };
        p
    }

    fn fetch(&self, ctx: &egui::Context, store: &Backend) {
        let (got, ctx2, id, image) = (self.got.clone(), ctx.clone(), self.id.clone(), self.shown == Shown::Image);
        let max = (ctx.input(|i| i.max_texture_side) as u32).min(4096);
        store.call(
            ctx,
            Method::Get,
            &format!("api/files/{id}"),
            None,
            Box::new(move |answer| match answer {
                Ok((s, bytes)) if s < 400 => {
                    if image {
                        decode(bytes, max, id, got, ctx2);
                    } else {
                        push(&got, &ctx2, id, Got::Bytes(keep_head(bytes)));
                    }
                }
                Ok((s, _)) => push(&got, &ctx2, id, Got::Failed(format!("The server said {s}."))),
                Err(e) => push(&got, &ctx2, id, Got::Failed(e)),
            }),
        );
    }

    fn take_got(&mut self, ctx: &egui::Context) {
        let got: Vec<_> = std::mem::take(&mut *self.got.lock().unwrap_or_else(|e| e.into_inner()));
        for (id, g) in got {
            if id != self.id {
                continue;
            }
            self.content = match g {
                Got::Image(Some(img)) => Content::Image(ctx.load_texture(format!("preview-{id}"), img, egui::TextureOptions::LINEAR)),
                Got::Image(None) => Content::Nothing("This picture couldn't be read.".into()),
                Got::Bytes(b) => Content::Text(b),
                Got::Failed(e) => Content::Nothing(e),
            };
        }
    }

    /// For the test hook.
    pub(super) fn test_state(&self) -> serde_json::Value {
        let content = match &self.content {
            Content::Loading => "loading".to_string(),
            Content::Image(t) => format!("image {}x{}", t.size()[0], t.size()[1]),
            Content::Text(b) => format!("text {} {}", textpreview::view_label(&self.name), b.len()),
            Content::Element => "element".to_string(),
            Content::Nothing(_) => "none".to_string(),
        };
        #[cfg(target_arch = "wasm32")]
        let element = self.element.as_ref().map(|e| e.tag.clone());
        #[cfg(not(target_arch = "wasm32"))]
        let element: Option<String> = None;
        serde_json::json!({
            "id": self.id, "name": self.name, "shown": self.shown.label(), "content": content, "element": element,
            "zoom": self.zoom, "info": self.info, "index": self.at.0, "count": self.at.1,
        })
    }

    pub(super) fn zoom_by(&mut self, f: f32) {
        self.zoom = (self.zoom * f).clamp(1.0, MAX_ZOOM);
        if self.zoom == 1.0 {
            self.pan = Vec2::ZERO;
        }
    }

    /// One frame. Takes every key while open.
    pub(super) fn ui(&mut self, ctx: &egui::Context) -> Option<Act> {
        self.take_got(ctx);
        let mut act = None;
        ctx.input_mut(|i| {
            if i.consume_key(Modifiers::NONE, Key::Escape) {
                act = Some(Act::Close);
            }
            // Esc heard by the page while a PDF frame or a film had the focus.
            // The pointer back over the canvas: take the keyboard back from a PDF frame, whose
            // keys never reach this page.
            #[cfg(target_arch = "wasm32")]
            {
                if self.element.as_ref().is_some_and(|e| e.esc.swap(false, Ordering::Relaxed)) {
                    act = Some(Act::Close);
                }
                if i.pointer.delta() != Vec2::ZERO
                    && let Some(e) = &self.element
                {
                    e.refocus_canvas();
                }
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowRight) {
                act = Some(Act::Step(1));
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowLeft) {
                act = Some(Act::Step(-1));
            }
            if self.shown == Shown::Image {
                if i.consume_key(Modifiers::NONE, Key::Plus) || i.consume_key(Modifiers::NONE, Key::Equals) {
                    self.zoom_by(1.5);
                }
                if i.consume_key(Modifiers::NONE, Key::Minus) {
                    self.zoom_by(1.0 / 1.5);
                }
                if i.consume_key(Modifiers::NONE, Key::Num0) {
                    self.zoom = 1.0;
                    self.pan = Vec2::ZERO;
                }
            }
        });
        let screen = ctx.content_rect();
        let mut area = Rect::NOTHING;
        egui::Area::new(egui::Id::unique("file_preview")).order(Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            let (rect, resp) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 0, Color32::from_black_alpha(235));
            let right = if self.info { INFO_W } else { 0.0 };
            area = Rect::from_min_max(pos2(rect.left() + 80.0, rect.top() + BAR + 8.0), pos2(rect.right() - 80.0 - right, rect.bottom() - 24.0));
            self.content_ui(ui, &resp, area, ctx);
            // The bar at the top.
            let bar = Rect::from_min_size(rect.min, vec2(rect.width(), BAR));
            let mut b =
                ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(16.0, 10.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
            if let Some(img) = crate::icons::icon_in(super::items::kind_icon(&self.name), Color32::WHITE) {
                b.add(img.fit_to_exact_size(vec2(24.0, 24.0)));
            }
            b.add_space(8.0);
            b.add(egui::Label::new(RichText::new(&self.name).size(16.0).color(Color32::WHITE)).truncate());
            b.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, "white_close", "Close (Esc)") {
                    act = Some(Act::Close);
                }
                if icon_toggle(ui, "white_info", "Details", self.info) {
                    self.info = !self.info;
                }
                if icon_button(ui, "white_download", "Download") {
                    act = Some(Act::Download(self.id.clone(), self.name.clone()));
                }
                ui.menu_button(RichText::new("Open with  ▾").size(14.0).color(Color32::WHITE), |ui| {
                    if cfg!(target_arch = "wasm32") && ui.button("A new tab (the browser's own viewer)").clicked() {
                        act = Some(Act::NewTab(self.id.clone()));
                        ui.close();
                    }
                    if ui.button("Download, then open on this computer").clicked() {
                        act = Some(Act::Download(self.id.clone(), self.name.clone()));
                        ui.close();
                    }
                });
                if self.at.1 > 1 {
                    ui.label(RichText::new(format!("{} of {}", self.at.0 + 1, self.at.1)).size(13.0).color(Color32::from_gray(200)));
                }
            });
            if self.info {
                self.info_ui(ui, Rect::from_min_max(pos2(rect.right() - INFO_W, rect.top() + BAR), rect.max));
            }
            if self.at.1 > 1 {
                for (icon, by, x) in [("white_chevron_left", -1, rect.left() + 40.0), ("white_chevron_right", 1, rect.right() - right - 40.0)] {
                    let r = Rect::from_center_size(pos2(x, area.center().y), vec2(48.0, 48.0));
                    let hit = ui.interact(r, egui::Id::unique(("preview_arrow", by)), Sense::click());
                    painter.circle_filled(r.center(), 24.0, Color32::from_white_alpha(if hit.hovered() { 60 } else { 30 }));
                    match crate::icons::icon(icon) {
                        Some(img) => img.paint_at(ui, Rect::from_center_size(r.center(), vec2(32.0, 32.0))),
                        None => {
                            painter.text(
                                r.center(),
                                egui::Align2::CENTER_CENTER,
                                if by < 0 { "‹" } else { "›" },
                                egui::FontId::proportional(28.0),
                                Color32::WHITE,
                            );
                        }
                    }
                    if hit.clicked() {
                        act = Some(Act::Step(by));
                    }
                }
            }
        });
        #[cfg(target_arch = "wasm32")]
        self.place_element(ctx, area);
        act
    }

    fn content_ui(&mut self, ui: &mut egui::Ui, resp: &egui::Response, area: Rect, ctx: &egui::Context) {
        let painter = ui.painter_at(area);
        let grey = Color32::from_gray(210);
        match &self.content {
            Content::Loading => {
                painter.text(area.center(), egui::Align2::CENTER_CENTER, "Loading…", egui::FontId::proportional(16.0), grey);
            }
            Content::Element => {}
            Content::Image(t) => {
                let t = t.clone();
                if resp.hovered() {
                    let pinch = ctx.input(|i| i.zoom_delta());
                    if pinch != 1.0 {
                        self.zoom_by(pinch);
                    }
                }
                if resp.dragged() && self.zoom > 1.0 {
                    self.pan += resp.drag_delta();
                }
                let [w, h] = t.size().map(|v| v as f32);
                let fit = (area.width() / w).min(area.height() / h).min(1.0);
                let size = vec2(w, h) * fit * self.zoom;
                let room = ((size - area.size()) / 2.0).max(Vec2::ZERO);
                self.pan = self.pan.clamp(-room, room);
                let r = Rect::from_center_size(area.center() + self.pan, size);
                ui.painter_at(area).image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                // Zoom buttons at the bottom.
                let zb = Rect::from_center_size(pos2(area.center().x, area.bottom() - 4.0), vec2(150.0, 44.0));
                ui.painter().rect_filled(zb, 22, Color32::from_black_alpha(200));
                let mut z = ui.new_child(egui::UiBuilder::new().max_rect(zb.shrink(2.0)).layout(egui::Layout::left_to_right(egui::Align::Center)));
                if icon_button(&mut z, "white_zoom_out", "Zoom out (−)") {
                    self.zoom_by(1.0 / 1.5);
                }
                z.label(RichText::new(format!("{:.0}%", self.zoom * 100.0)).size(13.0).color(Color32::WHITE));
                if icon_button(&mut z, "white_zoom_in", "Zoom in (+)") {
                    self.zoom_by(1.5);
                }
            }
            Content::Text(bytes) => {
                let bytes = bytes.clone();
                let mut t = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Min)));
                egui::Frame::NONE.fill(Color32::WHITE).corner_radius(8).inner_margin(16).show(&mut t, |ui| {
                    ui.set_min_size(area.size() - vec2(32.0, 32.0));
                    textpreview::show(ui, &self.name, &bytes);
                });
            }
            Content::Nothing(why) => {
                let why = why.clone();
                let card = Rect::from_center_size(area.center(), vec2(360.0, 180.0));
                ui.painter().rect_filled(card, 12, Color32::from_gray(48));
                let mut c = ui.new_child(egui::UiBuilder::new().max_rect(card.shrink(20.0)).layout(egui::Layout::top_down(egui::Align::Center)));
                c.label(RichText::new("No preview available").size(18.0).color(Color32::WHITE));
                if !why.is_empty() {
                    c.label(RichText::new(why).size(13.0).color(grey));
                }
                c.add_space(16.0);
                c.label(RichText::new("Use Download or Open with above.").size(13.0).color(grey));
            }
        }
    }

    fn info_ui(&self, ui: &mut egui::Ui, r: Rect) {
        ui.painter().rect_filled(r, 0, Color32::from_gray(32));
        let mut p = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink(20.0)).layout(egui::Layout::top_down(egui::Align::Min)));
        p.label(RichText::new("Details").size(16.0).color(Color32::WHITE));
        p.add_space(12.0);
        let date = chrono_like(self.modified);
        for (k, v) in [
            ("Type", super::items::file_kind(&self.name).to_string()),
            ("Size", super::size_text(self.size)),
            ("Modified", date),
            ("Owner", self.owner.clone()),
        ] {
            p.label(RichText::new(k).size(12.0).color(Color32::from_gray(170)));
            p.label(RichText::new(v).size(14.0).color(Color32::WHITE));
            p.add_space(8.0);
        }
    }
}

/// A modification time (Unix seconds) as a UTC date.
fn chrono_like(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    // Civil from days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn push(got: &Arc<Mutex<Vec<(String, Got)>>>, ctx: &egui::Context, id: String, g: Got) {
    got.lock().unwrap_or_else(|e| e.into_inner()).push((id, g));
    ctx.request_repaint();
}

/// Decodes a picture with Photos' decoders (EXIF orientation applied).
fn decode(bytes: Vec<u8>, max: u32, id: String, got: Arc<Mutex<Vec<(String, Got)>>>, ctx: egui::Context) {
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(async move {
        let img = super::photos::decode_web(bytes, max).await;
        push(&got, &ctx, id, Got::Image(img));
    });
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(move || {
        let img = super::photos::decode_native(&bytes, max);
        push(&got, &ctx, id, Got::Image(img));
    });
}

fn icon_button(ui: &mut egui::Ui, icon: &str, tip: &str) -> bool {
    icon_toggle(ui, icon, tip, false)
}

fn icon_toggle(ui: &mut egui::Ui, icon: &str, tip: &str, on: bool) -> bool {
    let b = match crate::icons::icon(icon) {
        Some(img) => egui::Button::image(img.fit_to_exact_size(vec2(24.0, 24.0))),
        None => egui::Button::new(RichText::new(tip).color(Color32::WHITE)),
    };
    let b = if on { b.fill(Color32::from_white_alpha(60)).corner_radius(20) } else { b.frame(false) };
    ui.add(b.min_size(vec2(40.0, 40.0))).on_hover_text(tip).clicked()
}

// ------------------------------------------------------------ the page element (web)

/// A page event listener kept alive by Rust.
#[cfg(target_arch = "wasm32")]
type Listener = wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)>;

/// A `<iframe>`, `<video>` or `<audio>` over the canvas, removed when dropped.
#[cfg(target_arch = "wasm32")]
struct Element {
    el: web_sys::HtmlElement,
    tag: String,
    /// Left, top, width, height in CSS pixels, and 1 while hidden for an open menu.
    at: [i32; 5],
    /// Set by the page-level Esc listeners; read (and cleared) by [`FilePreview::ui`].
    esc: Arc<AtomicBool>,
    /// Listeners added for this element (target, event; all capturing), removed with it.
    listeners: Vec<(web_sys::EventTarget, &'static str, Listener)>,
}

#[cfg(target_arch = "wasm32")]
impl Drop for Element {
    fn drop(&mut self) {
        use wasm_bindgen::JsCast;
        for (target, ev, f) in self.listeners.drain(..) {
            let _ = target.remove_event_listener_with_callback_and_bool(ev, f.as_ref().unchecked_ref(), true);
        }
        self.el.remove();
    }
}

#[cfg(target_arch = "wasm32")]
impl Element {
    /// Gives the keyboard back to egui's canvas when the PDF frame has it.
    fn refocus_canvas(&self) {
        refocus_canvas(&self.el);
    }

    /// Hears Esc on `target` (the page, or the PDF frame's own page when the browser lets us in).
    fn listen_esc(&mut self, target: web_sys::EventTarget, ctx: &egui::Context) {
        use wasm_bindgen::JsCast;
        let (esc, ctx) = (self.esc.clone(), ctx.clone());
        let f = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(move |e: web_sys::Event| {
            let Some(k) = e.dyn_ref::<web_sys::KeyboardEvent>() else { return };
            if closes_preview(&k.key(), k.ctrl_key(), k.alt_key(), k.meta_key(), k.shift_key()) {
                esc.store(true, Ordering::Relaxed);
                ctx.request_repaint();
            }
        });
        if target.add_event_listener_with_callback_and_bool("keydown", f.as_ref().unchecked_ref(), true).is_ok() {
            self.listeners.push((target, "keydown", f));
        }
    }
}

/// Focuses egui's canvas if `el` (the PDF frame) has the keyboard focus.
#[cfg(target_arch = "wasm32")]
fn refocus_canvas(el: &web_sys::HtmlElement) {
    use wasm_bindgen::JsCast;
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
    let focused = doc.active_element().is_some_and(|a| a == *el.unchecked_ref::<web_sys::Element>());
    if focused && let Some(c) = doc.get_element_by_id(crate::web::CANVAS_ID).and_then(|c| c.dyn_into::<web_sys::HtmlElement>().ok()) {
        let _ = c.focus();
    }
}

#[cfg(target_arch = "wasm32")]
impl FilePreview {
    fn place_element(&mut self, ctx: &egui::Context, area: Rect) {
        use wasm_bindgen::JsCast;
        if !matches!(self.content, Content::Element) {
            self.element = None;
            return;
        }
        if self.element.is_none() {
            let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
            let tag = match self.shown {
                Shown::Pdf => "iframe",
                Shown::Video => "video",
                _ => "audio",
            };
            let Ok(el) = doc.create_element(tag) else { return };
            let src = format!("api/files/{}/raw", self.id);
            let _ = el.set_attribute("src", &src);
            let _ = el.set_attribute("data-starbase-preview", &self.id);
            if tag != "iframe" {
                let _ = el.set_attribute("controls", "");
                let _ = el.set_attribute("autoplay", "");
            } else {
                let _ = el.set_attribute("title", &self.name);
            }
            let Ok(el) = el.dyn_into::<web_sys::HtmlElement>() else { return };
            if let Some(body) = doc.body() {
                let _ = body.append_child(&el);
            }
            let mut e = Element { el, tag: tag.into(), at: [-1; 5], esc: Arc::default(), listeners: Vec::new() };
            // Keys in a film's or sound's controls stay on this page: hear Esc there.
            if let Some(w) = web_sys::window() {
                e.listen_esc(w.into(), ctx);
            }
            // A PDF frame is its own page: when it has loaded, hear Esc inside it if the browser
            // lets us (the same site), and hand the keyboard back to the canvas either way.
            if tag == "iframe" {
                let (esc, ctx2, frame) = (e.esc.clone(), ctx.clone(), e.el.clone());
                let on_load = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
                    refocus_canvas(&frame);
                    let Some(inner) = frame.dyn_ref::<web_sys::HtmlIFrameElement>().and_then(|f| f.content_window()) else { return };
                    let (esc, ctx2) = (esc.clone(), ctx2.clone());
                    let f = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(move |e: web_sys::Event| {
                        let Some(k) = e.dyn_ref::<web_sys::KeyboardEvent>() else { return };
                        if closes_preview(&k.key(), k.ctrl_key(), k.alt_key(), k.meta_key(), k.shift_key()) {
                            esc.store(true, Ordering::Relaxed);
                            ctx2.request_repaint();
                        }
                    });
                    // Another origin (or the browser's own viewer) refuses: the pointer refocus covers it.
                    let target: &web_sys::EventTarget = inner.unchecked_ref();
                    if target.add_event_listener_with_callback_and_bool("keydown", f.as_ref().unchecked_ref(), true).is_ok() {
                        // Lives as long as the frame's page, which goes when the frame is removed.
                        f.forget();
                    }
                });
                let target: web_sys::EventTarget = e.el.clone().into();
                if target.add_event_listener_with_callback_and_bool("load", on_load.as_ref().unchecked_ref(), true).is_ok() {
                    e.listeners.push((target, "load", on_load));
                }
                // A click into the PDF moves the keyboard into the frame, and a browser's own PDF
                // viewer (Firefox's) never lets this page hear its keys, so Esc would do nothing.
                // As Google Drive does, keep the keyboard on the page: when the page loses it to
                // the frame, take it straight back (the wheel and clicks still reach the PDF).
                if let Some(w) = web_sys::window() {
                    let frame = e.el.clone();
                    let on_blur = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
                        let frame = frame.clone();
                        // The frame gets the focus just after the page's blur: look a moment later.
                        let later = wasm_bindgen::closure::Closure::once_into_js(move || refocus_canvas(&frame));
                        if let Some(w) = web_sys::window() {
                            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(later.unchecked_ref(), 0);
                        }
                    });
                    let target: web_sys::EventTarget = w.into();
                    if target.add_event_listener_with_callback_and_bool("blur", on_blur.as_ref().unchecked_ref(), true).is_ok() {
                        e.listeners.push((target, "blur", on_blur));
                    }
                }
            }
            self.element = Some(e);
        }
        let z = ctx.zoom_factor();
        let mut r = area;
        if self.shown == Shown::Audio {
            r = Rect::from_center_size(area.center(), vec2(area.width().min(480.0), 54.0));
        }
        let [x, y, w, h] = [r.left(), r.top(), r.width(), r.height()].map(|v| (v * z).round() as i32);
        // The page element sits above the canvas: while a menu is open (Open with drops into the
        // content area) hide it, or the menu would be drawn underneath it.
        let hidden = egui::Popup::is_any_open(ctx);
        let at = [x, y, w, h, hidden as i32];
        if let Some(e) = self.element.as_mut()
            && e.at != at
        {
            e.at = at;
            let bg = if self.shown == Shown::Pdf { "background:#fff;" } else { "background:#000;" };
            let _ = e.el.set_attribute(
                "style",
                &format!(
                    "position:fixed;left:{x}px;top:{y}px;width:{w}px;height:{h}px;border:0;z-index:10;{bg}{}",
                    if hidden { "visibility:hidden;" } else { "" }
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every extension Drive knows, and how its preview shows it.
    #[test]
    fn every_type_has_its_viewer() {
        let table: &[(&[&str], Shown)] = &[
            (&["png", "jpg", "jpeg", "jfif", "gif", "webp", "avif", "bmp", "ico"], Shown::Image),
            (&["pdf"], Shown::Pdf),
            (&["mp4", "m4v", "webm", "ogv", "mov"], Shown::Video),
            (&["mp3", "wav", "ogg", "oga", "opus", "flac", "m4a", "aac"], Shown::Audio),
            (
                &[
                    "txt", "text", "md", "markdown", "log", "json", "yaml", "yml", "toml", "ini", "cfg", "conf", "env", "tsv", "rs", "py", "js",
                    "mjs", "ts", "tsx", "jsx", "c", "h", "cc", "cpp", "hpp", "java", "kt", "go", "rb", "php", "sh", "bash", "zsh", "fish", "ps1",
                    "bat", "css", "scss", "sql", "lua", "pl", "r", "swift", "cs", "diff", "patch", "tex", "srt", "vtt", "nfo", "csv", "svg", "html",
                    "htm", "xml",
                ],
                Shown::Text,
            ),
            (&["zip", "exe", "iso", "7z", "psd", "", "xlsx", "docx", "odt", "pptx"], Shown::None),
        ];
        for (exts, want) in table {
            for e in *exts {
                for name in [format!("file.{e}"), format!("FILE.{}", e.to_uppercase())] {
                    assert_eq!(shown_as(&name), *want, "{name}");
                    // The desktop app shows only pictures and text itself.
                    let here = if cfg!(target_arch = "wasm32") || matches!(want, Shown::Image | Shown::Text) { *want } else { Shown::None };
                    assert_eq!(shown_as(&name).here(), here, "{name}");
                }
            }
        }
    }

    /// Esc closes the preview from wherever the page hears it; Esc with a modifier, and other
    /// keys, don't.
    #[test]
    fn esc_closes_from_the_page() {
        let table = [
            ("Escape", [false; 4], true),
            ("Esc", [false; 4], true),
            ("Escape", [true, false, false, false], false),
            ("Escape", [false, true, false, false], false),
            ("Escape", [false, false, true, false], false),
            ("Escape", [false, false, false, true], false),
            ("Enter", [false; 4], false),
            ("q", [false; 4], false),
            ("", [false; 4], false),
        ];
        for (key, [c, a, m, s], want) in table {
            assert_eq!(closes_preview(key, c, a, m, s), want, "{key} {c} {a} {m} {s}");
        }
    }

    /// Big text files keep only what the preview shows, plus one byte so it says so.
    #[test]
    fn text_keeps_only_the_head() {
        assert_eq!(keep_head(b"abc".to_vec()), b"abc");
        let big = keep_head(vec![b'x'; textpreview::MAX_BYTES + 500]);
        assert_eq!(big.len(), textpreview::MAX_BYTES + 1);
        assert!(TEXT_MAX as usize > textpreview::MAX_BYTES);
    }

    #[test]
    fn zoom_stays_in_range_and_fit_recentres() {
        let ctx = egui::Context::default();
        let store = Backend::default();
        let mut p = FilePreview::open(&ctx, &store, "x", "a.zip", 10, 0, String::new(), (0, 1));
        p.pan = vec2(5.0, 5.0);
        p.zoom_by(100.0);
        assert_eq!(p.zoom, MAX_ZOOM);
        p.zoom_by(0.0001);
        assert_eq!((p.zoom, p.pan), (1.0, Vec2::ZERO));
        assert!(matches!(p.content, Content::Nothing(_)));
    }

    #[test]
    fn dates() {
        assert_eq!(chrono_like(0), "1970-01-01");
        assert_eq!(chrono_like(1_764_000_000), "2025-11-24");
    }
}
