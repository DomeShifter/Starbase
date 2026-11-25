//! The photo editor (Photos job 3; the shared contract's §4), over the viewer in viewer.rs.
//!
//! - Opened from the viewer ("Edit", or E). One right-hand panel, as in Google Photos: **Crop**
//!   (aspect presets, rotate, flip, straighten, handles on the picture), **Adjust** (eight
//!   sliders; a double click puts one back), **Filters** (six looks and how strong).
//! - Non-destructive: Save writes the edits beside the picture (`PUT api/photos/edits/ID` with
//!   `If-Match`, the server keeps them in an XMP sidecar); the file itself never changes. "Save a
//!   copy…" makes a new, edited file (`POST api/photos/export/ID`).
//! - The live preview is the picture at 1024 px through `starbase-photo-edit`'s `render`
//!   (crates/photo-edit). It is
//!   rendered at most once a frame and only when the edits change; while a slider is dragged and
//!   a render takes longer than a frame, a half-size one stands in until the mouse is let go. The
//!   texture is replaced in place ([`egui::TextureHandle::set`]), never uploaded every frame.
//! - Its own undo history (Ctrl+Z, Ctrl+Shift+Z); hold \ (or the Compare button) to see the
//!   original. While it is open, every key is the editor's: nothing reaches the viewer or the
//!   Drive behind it.
//! - Light, like the Drive (the viewer behind it is black).

use std::sync::{Arc, Mutex};

use egui::{Align, Color32, CornerRadius, Key, Layout, Modifiers, Order, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Value as Json, json};

use super::photos::PhotoMode;
use crate::house;
use crate::store::{Backend, Method};
use crate::theme;

use starbase_photo_edit::{Aspect, Crop, Edits, Filter, FilterName, RgbaImage, output_size, render};

/// The longest side of the preview, and of the stand-in shown while dragging.
const PREVIEW: u32 = 1024;
const COARSE: u32 = 512;
/// The filter tiles' pictures.
const TILE: u32 = 96;
/// The right-hand panel, the bars.
const PANEL: f32 = 360.0;
const TOP: f32 = 56.0;
const BOTTOM: f32 = 64.0;
/// A render slower than this (ms) makes slider drags use the half-size stand-in.
const FRAME_MS: f64 = 12.0;
/// Where the crop handles can be grabbed (points from the edge).
const GRAB: f32 = 14.0;
/// The smallest crop, as a fraction of a side.
const MIN_CROP: f32 = 0.05;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tab {
    Crop,
    Adjust,
    Filters,
}

impl Tab {
    fn name(self) -> &'static str {
        match self {
            Tab::Crop => "crop",
            Tab::Adjust => "adjust",
            Tab::Filters => "filters",
        }
    }
    #[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code, reason = "the test hook's"))]
    fn from_name(s: &str) -> Option<Tab> {
        Some(match s {
            "crop" => Tab::Crop,
            "adjust" => Tab::Adjust,
            "filters" => Tab::Filters,
            _ => return None,
        })
    }
}

/// The filters offered, in the strip's order: (contract name, label).
pub(super) const FILTERS: [(&str, &str); 6] =
    [("vivid", "Vivid"), ("warm", "Warm"), ("cool", "Cool"), ("fade", "Fade"), ("bw", "Black & white"), ("sepia", "Sepia")];

/// The aspect presets: (contract name, label).
pub(super) const ASPECTS: [(&str, &str); 7] =
    [("free", "Free"), ("original", "Original"), ("1:1", "Square"), ("4:3", "4:3"), ("3:2", "3:2"), ("16:9", "16:9"), ("4:5", "4:5")];

/// One slider: its field in the edits, the label shown, its range.
pub(super) struct Knob {
    pub key: &'static str,
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub get: fn(&Edits) -> f32,
    pub set: fn(&mut Edits, f32),
}

/// The Adjust tab's sliders, in Google Photos' order.
pub(super) fn knobs() -> [Knob; 8] {
    fn i(v: f32) -> i32 {
        v.round().clamp(-100.0, 100.0) as i32
    }
    [
        Knob { key: "exposure", label: "Brightness", min: -5.0, max: 5.0, get: |e| e.exposure, set: |e, v| e.exposure = v.clamp(-5.0, 5.0) },
        Knob { key: "contrast", label: "Contrast", min: -100.0, max: 100.0, get: |e| e.contrast as f32, set: |e, v| e.contrast = i(v) },
        Knob { key: "highlights", label: "Highlights", min: -100.0, max: 100.0, get: |e| e.highlights as f32, set: |e, v| e.highlights = i(v) },
        Knob { key: "shadows", label: "Shadows", min: -100.0, max: 100.0, get: |e| e.shadows as f32, set: |e, v| e.shadows = i(v) },
        Knob { key: "saturation", label: "Saturation", min: -100.0, max: 100.0, get: |e| e.saturation as f32, set: |e, v| e.saturation = i(v) },
        Knob { key: "vibrance", label: "Vibrance", min: -100.0, max: 100.0, get: |e| e.vibrance as f32, set: |e, v| e.vibrance = i(v) },
        Knob { key: "temperature", label: "Warmth", min: -100.0, max: 100.0, get: |e| e.temperature as f32, set: |e, v| e.temperature = i(v) },
        Knob { key: "tint", label: "Tint", min: -100.0, max: 100.0, get: |e| e.tint as f32, set: |e, v| e.tint = i(v) },
    ]
}

/// A question over the editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Ask {
    /// Cancel with changes: throw them away?
    Discard,
    /// The first save over another program's edits (darktable).
    Foreign,
    /// 409: the edits were changed elsewhere since they were opened.
    Conflict,
    /// "Save a copy…": options.
    Copy,
}

impl Ask {
    fn name(&self) -> &'static str {
        match self {
            Ask::Discard => "discard",
            Ask::Foreign => "foreign",
            Ask::Conflict => "conflict",
            Ask::Copy => "copy",
        }
    }
}

/// Answers from the network and the decoder (possibly another thread on the desktop).
enum Got {
    /// GET edits: status and body.
    Loaded(Result<(u16, Vec<u8>), String>),
    Source(Option<egui::ColorImage>),
    /// PUT edits: status and body; whether a copy is to be made straight after.
    Saved(Result<(u16, Vec<u8>), String>),
    Copied(Result<(u16, Vec<u8>), String>),
}

/// What the editor did this frame, for the viewer and the grid.
#[derive(Debug, PartialEq)]
pub(super) enum Outcome {
    Open,
    Closed,
    /// Saved: the picture's new thumbnail version.
    Saved(Option<String>),
}

pub(super) struct Editor {
    pub id: String,
    pub name: String,
    pub tab: Tab,
    pub edits: Edits,
    /// The edits as last loaded or saved (dirty = they differ).
    saved: Edits,
    /// Undo history: the edits before each finished change.
    undo: Vec<Edits>,
    redo: Vec<Edits>,
    /// The edits when the last change finished (a drag is one change).
    settled: Edits,
    etag: Option<String>,
    pub loaded: bool,
    pub writable: bool,
    /// Why Save can't be used, when it can't.
    pub why_not: Option<String>,
    pub foreign: Vec<String>,
    /// The other program's edits were mentioned once already.
    warned: bool,
    pub saving: bool,
    pub ask: Option<Ask>,
    pub error: Option<String>,
    /// A short note in the bottom bar ("Saved a copy as …").
    pub message: Option<String>,
    /// The copy's options.
    pub strip_location: bool,
    pub max_side: Option<u32>,
    /// Hold to compare: the original is shown.
    pub comparing: bool,
    compare_button: bool,
    /// Compare held by the test hook (`compare {on}`); the button's state is re-read every frame.
    compare_held: bool,
    inbox: Arc<Mutex<Vec<Got>>>,
    src: Option<RgbaImage>,
    coarse: Option<RgbaImage>,
    tiny: Option<RgbaImage>,
    preview: Option<egui::TextureHandle>,
    original: Option<egui::TextureHandle>,
    /// What the preview texture shows: the edits rendered, and whether at half size.
    shown: Option<(Edits, bool)>,
    filter_tex: Vec<egui::TextureHandle>,
    filter_for: Option<Edits>,
    /// Time of the last render (ms) and how many there were, for the test hook.
    pub render_ms: f64,
    pub renders: u64,
    pub uploads: u64,
    /// A crop handle being dragged.
    grab: Option<Grab>,
    /// The folder should be listed again (a copy was made).
    pub relist: bool,
    outcome: Option<Outcome>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    Move,
    /// Edges moved: left, top, right, bottom.
    Edges(bool, bool, bool, bool),
}

impl Editor {
    /// A new editor on picture `id`; asks for its edits and the picture at 1024 px.
    pub fn open(ctx: &egui::Context, store: &Backend, id: &str, name: &str) -> Editor {
        let mut ed = Editor::empty(id, name);
        ed.load(ctx, store);
        ed.load_source(ctx, store);
        ed
    }

    /// An editor with nothing asked for yet (tests set the source and replies themselves).
    pub fn empty(id: &str, name: &str) -> Editor {
        Editor {
            id: id.to_string(),
            name: name.to_string(),
            tab: Tab::Adjust,
            edits: Edits::default(),
            saved: Edits::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            settled: Edits::default(),
            etag: None,
            loaded: false,
            writable: false,
            why_not: Some("Loading…".into()),
            foreign: Vec::new(),
            warned: false,
            saving: false,
            ask: None,
            error: None,
            message: None,
            strip_location: true,
            max_side: None,
            comparing: false,
            compare_button: false,
            compare_held: false,
            inbox: Arc::default(),
            src: None,
            coarse: None,
            tiny: None,
            preview: None,
            original: None,
            shown: None,
            filter_tex: Vec::new(),
            filter_for: None,
            render_ms: 0.0,
            renders: 0,
            uploads: 0,
            grab: None,
            relist: false,
            outcome: None,
        }
    }

    pub fn dirty(&self) -> bool {
        self.edits != self.saved
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || self.edits != self.settled
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn push(inbox: &Arc<Mutex<Vec<Got>>>, ctx: &egui::Context, g: Got) {
        inbox.lock().unwrap_or_else(|e| e.into_inner()).push(g);
        ctx.request_repaint();
    }

    fn load(&mut self, ctx: &egui::Context, store: &Backend) {
        let (inbox, ctx2) = (self.inbox.clone(), ctx.clone());
        store.call(ctx, Method::Get, &format!("api/photos/edits/{}", self.id), None, Box::new(move |a| Self::push(&inbox, &ctx2, Got::Loaded(a))));
    }

    /// The picture itself, made no bigger than 1024 px (the thumbnails show the edited view, so
    /// the editor starts from the original file). Both decoders apply the EXIF orientation, so
    /// `render` gets the upright picture it expects.
    fn load_source(&mut self, ctx: &egui::Context, store: &Backend) {
        let max = PREVIEW.min(ctx.input(|i| i.max_texture_side) as u32);
        let (inbox, ctx2) = (self.inbox.clone(), ctx.clone());
        store.call(
            ctx,
            Method::Get,
            &format!("api/files/{}/raw", self.id),
            None,
            Box::new(move |a| match a {
                Ok((200, bytes)) => decode(bytes, max, inbox, ctx2),
                _ => Self::push(&inbox, &ctx2, Got::Source(None)),
            }),
        );
    }

    /// Takes the picture to edit (the 1024 px one).
    pub fn set_source(&mut self, img: &egui::ColorImage) {
        let raw: Vec<u8> = img.pixels.iter().flat_map(|p| p.to_srgba_unmultiplied()).collect();
        let Some(src) = RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, raw) else { return };
        let small = |max: u32| {
            let (w, h) = src.dimensions();
            let s = (max as f32 / w.max(h) as f32).min(1.0);
            let (nw, nh) = (((w as f32 * s).round() as u32).max(1), ((h as f32 * s).round() as u32).max(1));
            image::imageops::resize(&src, nw, nh, image::imageops::FilterType::Triangle)
        };
        self.coarse = Some(small(COARSE));
        self.tiny = Some(small(TILE));
        self.src = Some(src);
        self.shown = None;
        self.original = None;
        self.filter_for = None;
    }

    /// The GET edits reply: `{"edits", "etag", "writable", "foreign"}`.
    fn take_loaded(&mut self, a: Result<(u16, Vec<u8>), String>) {
        self.loaded = true;
        match a {
            Ok((200, b)) => {
                let v: Json = serde_json::from_slice(&b).unwrap_or_default();
                match serde_json::from_value::<Edits>(v["edits"].clone()) {
                    Ok(e) => {
                        self.edits = e.clone();
                        self.saved = e.clone();
                        self.settled = e;
                    }
                    Err(_) => self.error = Some("This picture's saved edits couldn't be read; you are starting from the original.".into()),
                }
                self.etag = v["etag"].as_str().map(str::to_string);
                self.writable = v["writable"].as_bool().unwrap_or(false);
                self.foreign = v["foreign"].as_array().into_iter().flatten().filter_map(|f| f.as_str().map(str::to_string)).collect();
                self.why_not = (!self.writable).then(|| {
                    v["reason"].as_str().map_or("This picture's folder can't be changed, so edits can't be saved here.".to_string(), str::to_string)
                });
            }
            Ok((404, _)) => {
                self.writable = false;
                self.why_not = Some("This server can't keep picture edits yet.".into());
            }
            Ok((403, _)) => {
                self.writable = false;
                self.why_not = Some("You can view this picture but not change it.".into());
            }
            Ok((s, b)) => {
                let v: Json = serde_json::from_slice(&b).unwrap_or_default();
                self.writable = false;
                self.why_not = Some("The edits couldn't be loaded.".into());
                self.error = Some(v["error"].as_str().map_or(format!("The server said {s}."), str::to_string));
            }
            Err(e) => {
                self.writable = false;
                self.why_not = Some("The edits couldn't be loaded.".into());
                self.error = Some(e);
            }
        }
    }

    /// Answers since last frame.
    fn logic(&mut self, ctx: &egui::Context, store: &Backend) {
        let got: Vec<Got> = std::mem::take(&mut *self.inbox.lock().unwrap_or_else(|e| e.into_inner()));
        for g in got {
            match g {
                Got::Loaded(a) => self.take_loaded(a),
                Got::Source(Some(img)) => self.set_source(&img),
                Got::Source(None) => {
                    if self.src.is_none() {
                        self.error = Some("The picture couldn't be opened.".into());
                    }
                }
                Got::Saved(a) => {
                    self.saving = false;
                    match a {
                        Ok((200, b)) => {
                            let v: Json = serde_json::from_slice(&b).unwrap_or_default();
                            self.etag = v["etag"].as_str().map(str::to_string).or(self.etag.take());
                            self.saved = self.edits.clone();
                            self.outcome = Some(Outcome::Saved(v["thumb_v"].as_str().map(str::to_string)));
                        }
                        Ok((409 | 412, _)) => self.ask = Some(Ask::Conflict),
                        Ok((s, b)) => {
                            let v: Json = serde_json::from_slice(&b).unwrap_or_default();
                            self.error = Some(v["error"].as_str().map_or(format!("Not saved: the server said {s}."), str::to_string));
                        }
                        Err(e) => self.error = Some(format!("Not saved: {e}")),
                    }
                }
                Got::Copied(a) => {
                    self.saving = false;
                    match a {
                        Ok((200 | 201, b)) => {
                            let v: Json = serde_json::from_slice(&b).unwrap_or_default();
                            let path = v["path"].as_str().unwrap_or("");
                            let name = path.rsplit('/').next().unwrap_or(path);
                            self.message = Some(format!("Saved a copy as \u{201C}{name}\u{201D}"));
                            self.relist = true;
                        }
                        Ok((s, b)) => {
                            let v: Json = serde_json::from_slice(&b).unwrap_or_default();
                            self.error = Some(v["error"].as_str().map_or(format!("No copy made: the server said {s}."), str::to_string));
                        }
                        Err(e) => self.error = Some(format!("No copy made: {e}")),
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------ changes

    /// Ends a change (a drag counts once, when the mouse is let go): the edits before it go on
    /// the undo list.
    fn settle(&mut self, pointer_down: bool) {
        if pointer_down || self.edits == self.settled {
            return;
        }
        self.undo.push(std::mem::replace(&mut self.settled, self.edits.clone()));
        self.redo.clear();
        self.message = None;
    }

    pub fn do_undo(&mut self) {
        self.settle(false);
        if let Some(e) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.edits, e.clone()));
            self.settled = e;
        }
    }

    pub fn do_redo(&mut self) {
        self.settle(false);
        if let Some(e) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.edits, e.clone()));
            self.settled = e;
        }
    }

    /// Back to the original picture (as one step that can be undone).
    pub fn reset(&mut self) {
        self.edits = Edits::default();
        self.settle(false);
    }

    /// Cancel: closes, or asks first when there are changes.
    pub fn cancel(&mut self) {
        if self.dirty() {
            self.ask = Some(Ask::Discard);
        } else {
            self.outcome = Some(Outcome::Closed);
        }
    }

    /// Save: writes the edits beside the picture.
    pub fn save(&mut self, ctx: &egui::Context, store: &Backend) {
        self.save_then(ctx, store);
    }

    fn save_then(&mut self, ctx: &egui::Context, store: &Backend) {
        if !self.writable || self.saving {
            return;
        }
        if !self.foreign.is_empty() && !self.warned {
            self.ask = Some(Ask::Foreign);
            return;
        }
        if !self.dirty() {
            self.outcome = Some(Outcome::Closed);
            return;
        }
        // The sliders keep every field in range; this is the last check before anything is sent.
        if let Err(e) = self.edits.validate() {
            self.error = Some(format!("These edits can't be saved: {e}"));
            return;
        }
        self.saving = true;
        self.error = None;
        let body = serde_json::to_vec(&self.edits).unwrap_or_default();
        let (inbox, ctx2) = (self.inbox.clone(), ctx.clone());
        store.call_if(
            ctx,
            Method::Put,
            &format!("api/photos/edits/{}", self.id),
            Some(body),
            self.etag.clone(),
            Box::new(move |a| Self::push(&inbox, &ctx2, Got::Saved(a))),
        );
    }

    /// "Save a copy…" confirmed: the copy is made from the edits as they are now, sent in the
    /// request. Nothing is saved to the original's sidecar: it stays as it was before this session.
    pub fn copy(&mut self, ctx: &egui::Context, store: &Backend) {
        self.ask = None;
        if self.saving {
            return;
        }
        if let Err(e) = self.edits.validate() {
            self.error = Some(format!("No copy made: {e}"));
            return;
        }
        self.copy_now(ctx, store);
    }

    fn copy_now(&mut self, ctx: &egui::Context, store: &Backend) {
        self.saving = true;
        self.error = None;
        let body = json!({ "format": "jpeg", "quality": 90, "max_side": self.max_side, "strip_location": self.strip_location, "edits": self.edits });
        let (inbox, ctx2) = (self.inbox.clone(), ctx.clone());
        store.call(
            ctx,
            Method::Post,
            &format!("api/photos/export/{}", self.id),
            Some(body.to_string().into_bytes()),
            Box::new(move |a| Self::push(&inbox, &ctx2, Got::Copied(a))),
        );
    }

    /// Answers the question shown: `yes` = its main button.
    pub fn answer(&mut self, ctx: &egui::Context, store: &Backend, yes: bool) {
        let Some(ask) = self.ask.take() else { return };
        match (ask, yes) {
            (Ask::Discard, true) => self.outcome = Some(Outcome::Closed),
            (Ask::Foreign, true) => {
                self.warned = true;
                self.save(ctx, store);
            }
            (Ask::Conflict, true) => {
                // Reload: their edits replace these (this is a new start, so no undo across it).
                self.loaded = false;
                self.undo.clear();
                self.redo.clear();
                self.load(ctx, store);
            }
            (Ask::Copy, true) => self.copy(ctx, store),
            _ => {}
        }
    }

    /// Picks an aspect preset: the crop becomes the largest of that shape around its middle.
    pub fn set_aspect(&mut self, name: &str) {
        self.edits.aspect = aspect_from(name);
        if let Some(a) = self.aspect_ratio() {
            self.edits.crop = fit_aspect(self.edits.crop, a, self.frame_ratio());
        }
    }

    /// Turns the picture a quarter to the left, as Google Photos does (the crop turns with it).
    pub fn rotate(&mut self) {
        let e = &mut self.edits;
        // Shown = flip(rot(picture)): a turn to the left of what's shown is a turn to the right
        // underneath when it is flipped.
        e.rot90 = (e.rot90 + if e.flip_h { 1 } else { 3 }) % 4;
        let c = e.crop;
        e.crop = Crop { l: c.t, t: 1.0 - c.r, r: c.b, b: 1.0 - c.l };
        if let Some(a) = self.aspect_ratio() {
            self.edits.crop = fit_aspect(self.edits.crop, a, self.frame_ratio());
        }
    }

    pub fn flip(&mut self) {
        let e = &mut self.edits;
        e.flip_h = !e.flip_h;
        let c = e.crop;
        e.crop = Crop { l: 1.0 - c.r, r: 1.0 - c.l, ..c };
    }

    /// Width / height of the picture turned and straightened (what the crop is a part of).
    fn frame_ratio(&self) -> f32 {
        let (w, h) = self.src.as_ref().map_or((4, 3), |s| s.dimensions());
        let (w, h) = output_size(w, h, &uncropped(&self.edits));
        w.max(1) as f32 / h.max(1) as f32
    }

    /// The chosen aspect as width / height, None for free.
    fn aspect_ratio(&self) -> Option<f32> {
        let r = match self.edits.aspect? {
            Aspect::Free => return None,
            Aspect::Original => {
                let (w, h) = self.src.as_ref().map_or((4, 3), |s| s.dimensions());
                let r = w.max(1) as f32 / h.max(1) as f32;
                if self.edits.rot90 % 2 == 1 { 1.0 / r } else { r }
            }
            Aspect::Square => 1.0,
            Aspect::R4x3 => 4.0 / 3.0,
            Aspect::R3x2 => 1.5,
            Aspect::R16x9 => 16.0 / 9.0,
            Aspect::R4x5 => 0.8,
        };
        Some(r)
    }

    /// Sets fields from JSON (the test hook and tests): `{exposure: 1.2, contrast: 20, filter:
    /// {name, amount}, …}`; one change on the undo list.
    #[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code, reason = "the test hook's"))]
    pub fn set_json(&mut self, params: &Json) -> Result<(), String> {
        let mut v = serde_json::to_value(&self.edits).map_err(|e| e.to_string())?;
        let (Some(obj), Some(p)) = (v.as_object_mut(), params.as_object()) else { return Err("needs an object of edit fields".into()) };
        for (k, val) in p {
            if !obj.contains_key(k) {
                return Err(format!("unknown edit field `{k}`"));
            }
            obj.insert(k.clone(), val.clone());
        }
        let e: Edits = serde_json::from_value(v).map_err(|e| format!("not valid edits: {e}"))?;
        self.edits = e;
        self.settle(false);
        Ok(())
    }

    /// The edits the preview shows: none while comparing; in the Crop tab the whole picture
    /// (the crop is drawn over it).
    fn view_edits(&self) -> Edits {
        if self.tab == Tab::Crop { uncropped(&self.edits) } else { self.edits.clone() }
    }

    /// Renders the preview when the edits changed (at most once a frame), at half size while a
    /// drag is going on and a full render is too slow for a frame.
    fn update_preview(&mut self, ctx: &egui::Context, dragging: bool) {
        let Some(src) = &self.src else { return };
        if self.original.is_none() {
            self.original = Some(ctx.load_texture(format!("photo-edit-original-{}", self.id), to_color_image(src), egui::TextureOptions::LINEAR));
            self.uploads += 1;
        }
        let want = self.view_edits();
        let coarse = dragging && self.render_ms > FRAME_MS;
        let stale = match &self.shown {
            None => true,
            Some((e, was_coarse)) => *e != want || (*was_coarse && !coarse),
        };
        if !stale {
            return;
        }
        let from = if coarse { self.coarse.as_ref().unwrap_or(src) } else { src };
        let t0 = now_ms();
        let img = to_color_image(&render(from, &want));
        let ms = now_ms() - t0;
        if !coarse {
            self.render_ms = ms;
        }
        self.renders += 1;
        match &mut self.preview {
            Some(t) => t.set(img, egui::TextureOptions::LINEAR),
            None => self.preview = Some(ctx.load_texture(format!("photo-edit-{}", self.id), img, egui::TextureOptions::LINEAR)),
        }
        self.uploads += 1;
        self.shown = Some((want, coarse));
    }

    /// The filter tiles, made again only when the other edits change.
    fn update_filter_tiles(&mut self, ctx: &egui::Context) {
        let Some(tiny) = &self.tiny else { return };
        let mut base = uncropped(&self.edits);
        base.filter = None;
        base.crop = self.edits.crop;
        if self.filter_for.as_ref() == Some(&base) && self.filter_tex.len() == FILTERS.len() {
            return;
        }
        let imgs: Vec<egui::ColorImage> = FILTERS
            .iter()
            .map(|(name, _)| {
                let mut e = base.clone();
                e.filter = filter_from(name).map(|name| Filter { name, amount: 1.0 });
                to_color_image(&render(tiny, &e))
            })
            .collect();
        if self.filter_tex.len() == imgs.len() {
            for (t, img) in self.filter_tex.iter_mut().zip(imgs) {
                t.set(img, egui::TextureOptions::LINEAR);
            }
        } else {
            self.filter_tex = imgs
                .into_iter()
                .enumerate()
                .map(|(i, img)| ctx.load_texture(format!("photo-edit-filter-{i}"), img, egui::TextureOptions::LINEAR))
                .collect();
        }
        self.uploads += FILTERS.len() as u64;
        self.filter_for = Some(base);
    }

    // ------------------------------------------------------------ drawing

    /// Keys, answers, drawing: one frame of the editor.
    pub fn ui(&mut self, ctx: &egui::Context, store: &Backend) -> Outcome {
        self.logic(ctx, store);
        // Every key is the editor's while it's open: its own first, then the rest are dropped so
        // nothing behind (the viewer, the Drive) sees them. A question takes Enter and Esc.
        let mut held_compare = false;
        let asking = self.ask.is_some();
        ctx.input_mut(|i| {
            if !asking {
                if i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || i.consume_key(Modifiers::COMMAND, Key::Y) {
                    self.do_redo();
                }
                if i.consume_key(Modifiers::COMMAND, Key::Z) {
                    self.do_undo();
                }
                if i.consume_key(Modifiers::NONE, Key::Escape) {
                    self.cancel();
                }
            }
            held_compare = i.key_down(Key::Backslash);
        });
        self.comparing = held_compare || self.compare_button || self.compare_held;
        let dragging = ctx.input(|i| i.pointer.any_down());
        self.update_preview(ctx, dragging);
        if self.tab == Tab::Filters {
            self.update_filter_tiles(ctx);
        }
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::unique("photo_edit")).order(Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(rect, 0, theme::PAGE);
            let panel = Rect::from_min_max(pos2(rect.right() - PANEL, rect.top() + TOP), pos2(rect.right(), rect.bottom() - BOTTOM));
            let top = Rect::from_min_size(rect.min, vec2(rect.width(), TOP));
            let bottom = Rect::from_min_max(pos2(rect.left(), rect.bottom() - BOTTOM), rect.max);
            let stage = Rect::from_min_max(pos2(rect.left(), top.bottom()), pos2(panel.left(), bottom.top()));
            self.top_ui(ui, top);
            self.stage_ui(ui, stage);
            self.panel_ui(ui, panel);
            self.bottom_ui(ui, bottom, ctx, store);
        });
        self.ask_ui(ctx, store);
        self.settle(ctx.input(|i| i.pointer.any_down()));
        ctx.input_mut(|i| {
            i.events.retain(|e| {
                !matches!(e, egui::Event::Key { .. } | egui::Event::Text(_) | egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_))
            })
        });
        self.outcome.take().unwrap_or(Outcome::Open)
    }

    fn top_ui(&mut self, ui: &mut Ui, top: Rect) {
        ui.painter().rect_filled(top, 0, Color32::WHITE);
        ui.painter().hline(top.x_range(), top.bottom(), Stroke::new(1.0, theme::DIVIDER));
        let mut bar =
            ui.new_child(egui::UiBuilder::new().max_rect(top.shrink2(vec2(house::M, house::S))).layout(Layout::left_to_right(Align::Center)));
        if house::close_button(&mut bar) {
            self.cancel();
        }
        bar.add_space(house::S);
        bar.label(RichText::new("Edit").size(18.0).color(theme::TEXT));
        bar.add_space(house::M);
        bar.add(egui::Label::new(RichText::new(&self.name).size(house::BODY).color(theme::TEXT_DIM)).truncate());
    }

    /// The picture: fitted in the middle, the crop over it in the Crop tab.
    fn stage_ui(&mut self, ui: &mut Ui, stage: Rect) {
        let painter = ui.painter_at(stage);
        painter.rect_filled(stage, 0, theme::MENU_HOVER);
        let area = stage.shrink(house::XL + if self.tab == Tab::Crop { GRAB } else { 0.0 });
        let tex = if self.comparing { self.original.as_ref() } else { self.preview.as_ref() };
        let Some(tex) = tex else {
            let text = if self.error.is_some() && self.src.is_none() { "" } else { "Loading…" };
            painter.text(stage.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(16.0), theme::TEXT_DIM);
            return;
        };
        let [w, h] = tex.size();
        let (w, h) = (w.max(1) as f32, h.max(1) as f32);
        let fit = (area.width() / w).min(area.height() / h);
        let r = Rect::from_center_size(area.center(), vec2(w, h) * fit);
        painter.image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        if self.comparing {
            let label = Rect::from_min_size(r.min + vec2(house::S, house::S), vec2(72.0, 24.0));
            painter.rect_filled(label, 12, Color32::from_black_alpha(150));
            painter.text(label.center(), egui::Align2::CENTER_CENTER, "Original", egui::FontId::proportional(house::NOTE), Color32::WHITE);
        } else if self.tab == Tab::Crop {
            self.crop_ui(ui, r);
        }
    }

    /// The crop box over the picture at `r`: the rest dimmed, thirds lines, handles to drag.
    fn crop_ui(&mut self, ui: &mut Ui, r: Rect) {
        let c = self.edits.crop;
        let at = |x: f32, y: f32| pos2(r.left() + x * r.width(), r.top() + y * r.height());
        let cr = Rect::from_min_max(at(c.l, c.t), at(c.r, c.b));
        let p = ui.painter();
        let dim = Color32::from_black_alpha(110);
        for d in [
            Rect::from_min_max(r.min, pos2(r.right(), cr.top())),
            Rect::from_min_max(pos2(r.left(), cr.bottom()), r.max),
            Rect::from_min_max(pos2(r.left(), cr.top()), pos2(cr.left(), cr.bottom())),
            Rect::from_min_max(pos2(cr.right(), cr.top()), pos2(r.right(), cr.bottom())),
        ] {
            p.rect_filled(d, 0, dim);
        }
        let thin = Stroke::new(1.0, Color32::from_white_alpha(140));
        for k in [1.0 / 3.0, 2.0 / 3.0] {
            p.vline(cr.left() + cr.width() * k, cr.y_range(), thin);
            p.hline(cr.x_range(), cr.top() + cr.height() * k, thin);
        }
        p.rect_stroke(cr, 0, Stroke::new(1.5, Color32::WHITE), egui::StrokeKind::Inside);
        for corner in [cr.left_top(), cr.right_top(), cr.left_bottom(), cr.right_bottom()] {
            let sx = if corner.x == cr.left() { 1.0 } else { -1.0 };
            let sy = if corner.y == cr.top() { 1.0 } else { -1.0 };
            let thick = Stroke::new(4.0, Color32::WHITE);
            p.line_segment([corner, corner + vec2(18.0 * sx, 0.0)], thick);
            p.line_segment([corner, corner + vec2(0.0, 18.0 * sy)], thick);
        }
        let resp = ui.interact(cr.expand(GRAB), egui::Id::unique("photo_edit_crop"), Sense::drag());
        if let Some(pos) = resp.hover_pos().or(resp.interact_pointer_pos()) {
            let g = self.grab.unwrap_or_else(|| grab_at(cr, pos));
            ui.ctx().set_cursor_icon(match g {
                Grab::Move => egui::CursorIcon::Grab,
                Grab::Edges(l, t, rr, b) if (l && t) || (rr && b) => egui::CursorIcon::ResizeNwSe,
                Grab::Edges(l, t, rr, b) if (rr && t) || (l && b) => egui::CursorIcon::ResizeNeSw,
                Grab::Edges(l, _, rr, _) if l || rr => egui::CursorIcon::ResizeHorizontal,
                _ => egui::CursorIcon::ResizeVertical,
            });
        }
        if resp.drag_started()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            self.grab = Some(grab_at(cr, pos));
        }
        if resp.dragged()
            && let Some(g) = self.grab
        {
            let d = resp.drag_delta();
            let (dx, dy) = (d.x / r.width().max(1.0), d.y / r.height().max(1.0));
            self.edits.crop = drag_crop(self.edits.crop, g, dx, dy, self.aspect_ratio(), r.width() / r.height().max(1.0));
        }
        if resp.drag_stopped() {
            self.grab = None;
        }
    }

    fn panel_ui(&mut self, ui: &mut Ui, panel: Rect) {
        ui.painter().rect_filled(panel, 0, Color32::WHITE);
        ui.painter().vline(panel.left(), panel.y_range(), Stroke::new(1.0, theme::DIVIDER));
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(panel.shrink(house::L)).layout(Layout::top_down(Align::Min)));
        house::form_style(&mut ui);
        let mut tab = [Tab::Crop, Tab::Adjust, Tab::Filters].iter().position(|t| *t == self.tab).unwrap_or(1);
        if house::tabs(&mut ui, &mut tab, &["Crop", "Adjust", "Filters"]) {
            self.tab = [Tab::Crop, Tab::Adjust, Tab::Filters][tab];
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(&mut ui, |ui| {
            ui.set_width(ui.available_width() - house::SCROLL_ROOM);
            match self.tab {
                Tab::Crop => self.crop_panel(ui),
                Tab::Adjust => self.adjust_panel(ui),
                Tab::Filters => self.filters_panel(ui),
            }
        });
    }

    fn crop_panel(&mut self, ui: &mut Ui) {
        house::caps(ui, "Aspect ratio");
        let current = self.edits.aspect.map_or_else(|| "free".to_string(), name_of);
        ui.horizontal_wrapped(|ui| {
            for (name, label) in ASPECTS {
                if chip(ui, label, current == name) {
                    self.set_aspect(name);
                }
            }
        });
        house::gap(ui);
        house::caps(ui, "Rotate and flip");
        ui.horizontal(|ui| {
            if ui.add(house::secondary_in("Rotate 90°", super::ACCENT)).on_hover_text("Turn a quarter to the left").clicked() {
                self.rotate();
            }
            if ui.add(house::secondary_in("Flip", super::ACCENT)).on_hover_text("Mirror left to right").clicked() {
                self.flip();
            }
        });
        house::gap(ui);
        let mut angle = self.edits.angle;
        if slider_row(ui, "Straighten", &mut angle, -45.0, 45.0, 0.0, |v| format!("{v:+.1}°")) {
            self.edits.angle = angle.clamp(-45.0, 45.0);
        }
        house::gap(ui);
        house::note(ui, "Drag the corners or edges of the box on the picture to crop it, or drag inside it to move it.");
    }

    fn adjust_panel(&mut self, ui: &mut Ui) {
        for k in knobs() {
            let mut v = (k.get)(&self.edits);
            let exposure = k.key == "exposure";
            let fmt = move |v: f32| shown(exposure, v);
            if slider_row(ui, k.label, &mut v, k.min, k.max, 0.0, fmt) {
                (k.set)(&mut self.edits, v);
            }
            ui.add_space(house::XS);
        }
        // Room under the last slider so it never sits against the panel's edge.
        ui.add_space(house::XL);
    }

    fn filters_panel(&mut self, ui: &mut Ui) {
        let current = self.edits.filter.map(|f| name_of(f.name));
        let side = ((ui.available_width() - 2.0 * house::S) / 3.0).floor();
        let mut picked: Option<Option<&str>> = None;
        let mut row = |ui: &mut Ui, items: &[(Option<&'static str>, &'static str, Option<usize>)]| {
            ui.horizontal(|ui| {
                for (name, label, tex) in items {
                    let on = current.as_deref() == *name;
                    ui.vertical(|ui| {
                        ui.set_width(side);
                        let (r, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click());
                        let p = ui.painter();
                        let t = match tex {
                            Some(i) => self.filter_tex.get(*i),
                            None => self.original.as_ref(),
                        };
                        p.rect_filled(r, 8, theme::MENU_HOVER);
                        if let Some(t) = t {
                            let [w, h] = t.size();
                            p.image(t.id(), r, super::photos::cover_uv(w as f32, h as f32), Color32::WHITE);
                        }
                        if on {
                            p.rect_stroke(r, 8, Stroke::new(3.0, super::ACCENT), egui::StrokeKind::Inside);
                        } else if resp.hovered() {
                            p.rect_stroke(r, 8, Stroke::new(1.0, theme::BOX), egui::StrokeKind::Inside);
                        }
                        let resp = resp.on_hover_text(*label);
                        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, *label));
                        let colour = if on { super::ACCENT } else { theme::TEXT };
                        let (cap, _) = ui.allocate_exact_size(vec2(side, 18.0), Sense::hover());
                        ui.painter().text(cap.left_center(), egui::Align2::LEFT_CENTER, *label, egui::FontId::proportional(house::NOTE), colour);
                        if resp.clicked() {
                            picked = Some(*name);
                        }
                    });
                }
            });
        };
        let mut all: Vec<(Option<&'static str>, &'static str, Option<usize>)> = vec![(None, "None", None)];
        all.extend(FILTERS.iter().enumerate().map(|(i, (n, l))| (Some(*n), *l, Some(i))));
        for chunk in all.chunks(3) {
            row(ui, chunk);
        }
        if let Some(p) = picked {
            let amount = self.edits.filter.map_or(1.0, |f| f.amount);
            self.edits.filter = p.and_then(filter_from).map(|name| Filter { name, amount });
        }
        if let Some(f) = &mut self.edits.filter {
            house::gap(ui);
            let mut a = f.amount * 100.0;
            if slider_row(ui, "Strength", &mut a, 0.0, 100.0, 100.0, |v| format!("{}", v.round() as i32)) {
                f.amount = (a / 100.0).clamp(0.0, 1.0);
            }
        }
    }

    fn bottom_ui(&mut self, ui: &mut Ui, bottom: Rect, ctx: &egui::Context, store: &Backend) {
        ui.painter().rect_filled(bottom, 0, Color32::WHITE);
        ui.painter().hline(bottom.x_range(), bottom.top(), Stroke::new(1.0, theme::DIVIDER));
        let mut bar =
            ui.new_child(egui::UiBuilder::new().max_rect(bottom.shrink2(vec2(house::L, house::S))).layout(Layout::left_to_right(Align::Center)));
        house::form_style(&mut bar);
        if icon_button(&mut bar, "undo", "Undo (Ctrl+Z)", self.can_undo()) {
            self.do_undo();
        }
        if icon_button(&mut bar, "redo", "Redo (Ctrl+Shift+Z)", self.can_redo()) {
            self.do_redo();
        }
        // Hold to compare: the original while the button is held down.
        let text = RichText::new("Compare").size(house::BODY).color(theme::TEXT);
        let b = match crate::icons::icon("visibility") {
            Some(img) => egui::Button::image_and_text(img, text),
            None => egui::Button::new(text),
        };
        let cmp = bar
            .add(
                b.fill(if self.comparing { house::SELECTED } else { Color32::WHITE })
                    .stroke(Stroke::new(1.0, theme::BOX))
                    .corner_radius(CornerRadius::same(house::BUTTON_RADIUS))
                    .min_size(vec2(0.0, house::BUTTON_HEIGHT))
                    .sense(Sense::click_and_drag()),
            )
            .on_hover_text("Hold to see the original (\\)");
        self.compare_button = cmp.is_pointer_button_down_on();
        if bar
            .add_enabled(!self.edits.is_identity(), house::secondary_in("Reset", super::ACCENT))
            .on_hover_text("Back to the original picture")
            .clicked()
        {
            self.reset();
        }
        if let Some(e) = self.error.clone() {
            bar.add_space(house::M);
            bar.add(egui::Label::new(RichText::new(e).size(house::NOTE).color(house::ERROR)).truncate());
        } else if let Some(m) = &self.message {
            bar.add_space(house::M);
            bar.add(egui::Label::new(RichText::new(m).size(house::NOTE).color(theme::TEXT_DIM)).truncate());
        }
        bar.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = house::S;
            let save = if self.saving { "Saving…" } else { "Save" };
            let ok = self.writable && !self.saving;
            let b = if ok { house::primary_in(save, super::ACCENT) } else { house::primary_off(save) };
            let r = ui.add_enabled(ok, b);
            let r = match &self.why_not {
                Some(why) if !self.writable => r.on_disabled_hover_text(why.as_str()),
                _ => r.on_hover_text("Keep these edits (the picture file itself doesn't change)"),
            };
            if r.clicked() {
                self.save(ctx, store);
            }
            if ui.add_enabled(self.writable && !self.saving, house::secondary_in("Save a copy…", super::ACCENT)).clicked() {
                self.ask = Some(Ask::Copy);
            }
            if ui.add(house::secondary_in("Cancel", super::ACCENT)).clicked() {
                self.cancel();
            }
            if let Some(why) = self.why_not.as_ref().filter(|_| !self.writable && self.loaded) {
                ui.add(egui::Label::new(RichText::new(why).size(house::NOTE).color(theme::TEXT_DIM)).truncate());
            }
        });
    }

    fn ask_ui(&mut self, ctx: &egui::Context, store: &Backend) {
        let Some(ask) = self.ask.clone() else { return };
        let mut close = false;
        let (title, body, labels): (&str, String, [&str; 2]) = match ask {
            Ask::Discard => {
                ("Discard your changes?", "The edits you made since opening this picture will be lost.".into(), ["Keep editing", "Discard"])
            }
            Ask::Foreign => (
                "Edited in another program",
                format!(
                    "This picture also has edits from {}. Starbase keeps its own edits beside them: {} won't show Starbase's changes, and Starbase doesn't show its.",
                    self.foreign.join(", "),
                    if self.foreign.len() == 1 { self.foreign[0].as_str() } else { "those programs" }
                ),
                ["Cancel", "Save anyway"],
            ),
            Ask::Conflict => (
                "Changed somewhere else",
                "Someone changed this picture's edits since you opened it. Reload theirs? Your changes here will be lost.".into(),
                ["Keep editing", "Reload"],
            ),
            Ask::Copy => ("Save a copy", String::new(), ["Cancel", "Save copy"]),
        };
        let hit = house::dialog(ctx, "photo_edit_ask", 440.0, title, Some(&mut close), |ui| {
            if ask == Ask::Copy {
                house::text(ui, "A new JPEG picture with these edits, next to this one. The original stays as it is.");
                if self.dirty() {
                    house::note(ui, "Your edits are saved first.");
                }
                house::gap(ui);
                ui.checkbox(&mut self.strip_location, RichText::new("Leave out where it was taken (recommended for sharing)").size(house::BODY));
                house::gap(ui);
                house::label(ui, "Size", "");
                let sizes = [None, Some(4096), Some(2048), Some(1024)];
                let mut k = sizes.iter().position(|s| *s == self.max_side).unwrap_or(0);
                if house::select(ui, "photo_edit_size", 240.0, &mut k, &["Full size", "4096 px", "2048 px", "1024 px"], &[]) {
                    self.max_side = sizes[k];
                }
            } else {
                house::text(ui, &body);
            }
            house::actions_in(ui, &labels, super::ACCENT)
        });
        let enter = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        if close || hit == Some(0) || ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.ask = None;
        } else if hit == Some(1) || enter {
            self.answer(ctx, store, true);
        }
    }
}

/// An aspect preset from its contract name ("1:1", "free", …).
fn aspect_from(name: &str) -> Option<Aspect> {
    serde_json::from_value(json!(name)).ok()
}

/// A filter from its contract name ("bw", "sepia", …).
fn filter_from(name: &str) -> Option<FilterName> {
    serde_json::from_value(json!(name)).ok()
}

/// The contract name of an aspect or filter.
fn name_of<T: serde::Serialize>(v: T) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// The edits without the crop (the Crop tab shows the whole picture with the box over it).
fn uncropped(e: &Edits) -> Edits {
    let mut v = e.clone();
    v.crop = Crop::default();
    v
}

/// The largest crop of shape `aspect` (width / height in pixels) around `c`'s middle, inside the
/// picture, whose own shape is `frame` (width / height).
fn fit_aspect(c: Crop, aspect: f32, frame: f32) -> Crop {
    // In fractions, a crop w × h has the pixel shape (w / h) · frame.
    let want = aspect / frame.max(1e-6);
    let (cx, cy) = ((c.l + c.r) / 2.0, (c.t + c.b) / 2.0);
    let (mut w, mut h) = (c.r - c.l, c.b - c.t);
    if w / h > want {
        w = h * want;
    } else {
        h = w / want;
    }
    // Never bigger than the picture.
    if w > 1.0 {
        h /= w;
        w = 1.0;
    }
    if h > 1.0 {
        w /= h;
        h = 1.0;
    }
    let l = (cx - w / 2.0).clamp(0.0, 1.0 - w);
    let t = (cy - h / 2.0).clamp(0.0, 1.0 - h);
    Crop { l, t, r: l + w, b: t + h }
}

/// Which part of the crop box `pos` grabs: an edge or corner near the border, else the inside.
fn grab_at(cr: Rect, pos: egui::Pos2) -> Grab {
    let l = (pos.x - cr.left()).abs() <= GRAB;
    let r = !l && (pos.x - cr.right()).abs() <= GRAB;
    let t = (pos.y - cr.top()).abs() <= GRAB;
    let b = !t && (pos.y - cr.bottom()).abs() <= GRAB;
    if l || r || t || b { Grab::Edges(l, t, r, b) } else { Grab::Move }
}

/// The crop after dragging `g` by (`dx`, `dy`) (fractions of the picture), keeping `aspect`
/// (pixels) when one is chosen; `frame` is the picture's own width / height.
fn drag_crop(c: Crop, g: Grab, dx: f32, dy: f32, aspect: Option<f32>, frame: f32) -> Crop {
    let mut n = c;
    match g {
        Grab::Move => {
            let (w, h) = (c.r - c.l, c.b - c.t);
            n.l = (c.l + dx).clamp(0.0, 1.0 - w);
            n.t = (c.t + dy).clamp(0.0, 1.0 - h);
            n.r = n.l + w;
            n.b = n.t + h;
            return n;
        }
        Grab::Edges(l, t, r, b) => {
            if l {
                n.l = (c.l + dx).clamp(0.0, c.r - MIN_CROP);
            }
            if r {
                n.r = (c.r + dx).clamp(c.l + MIN_CROP, 1.0);
            }
            if t {
                n.t = (c.t + dy).clamp(0.0, c.b - MIN_CROP);
            }
            if b {
                n.b = (c.b + dy).clamp(c.t + MIN_CROP, 1.0);
            }
            let Some(a) = aspect else { return n };
            // Keep the shape: the side dragged leads, the other follows (from the fixed edge).
            let want = a / frame.max(1e-6);
            let (w, h) = (n.r - n.l, n.b - n.t);
            if (l || r) && !(t || b) || ((l || r) && (t || b) && dx.abs() >= dy.abs()) {
                let mut nh = w / want;
                if t {
                    nh = nh.min(n.b);
                    n.t = n.b - nh;
                } else {
                    nh = nh.min(1.0 - n.t);
                    n.b = n.t + nh;
                }
                let nw = nh * want;
                if l { n.l = n.r - nw } else { n.r = n.l + nw }
            } else {
                let mut nw = h * want;
                if l {
                    nw = nw.min(n.r);
                    n.l = n.r - nw;
                } else {
                    nw = nw.min(1.0 - n.l);
                    n.r = n.l + nw;
                }
                let nh = nw / want;
                if t { n.t = n.b - nh } else { n.b = n.t + nh }
            }
        }
    }
    n
}

/// What an Adjust slider shows: every one on the same -100..+100 scale. Brightness is stored in
/// stops (-5..+5), so it is shown times 20.
fn shown(exposure: bool, v: f32) -> String {
    let v = if exposure { v * 20.0 } else { v };
    format!("{:+}", v.round() as i32)
}

/// A slider row: the label, its value at the right, the slider under them. A double click puts
/// it back to `default`. True when the value changed.
fn slider_row(ui: &mut Ui, label: &str, v: &mut f32, min: f32, max: f32, default: f32, fmt: impl Fn(f32) -> String) -> bool {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(house::BODY).color(theme::TEXT));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(fmt(*v)).size(house::NOTE).color(theme::TEXT_DIM));
        });
    });
    let before = *v;
    let w = ui.available_width();
    let r = ui.scope(|ui| {
        ui.spacing_mut().slider_width = w;
        let vis = &mut ui.visuals_mut().widgets;
        for s in [&mut vis.inactive, &mut vis.hovered, &mut vis.active] {
            s.bg_fill = super::ACCENT;
            s.fg_stroke = Stroke::new(1.0, super::ACCENT);
        }
        ui.visuals_mut().selection.bg_fill = super::ACCENT;
        ui.add(egui::Slider::new(v, min..=max).show_value(false).trailing_fill(true).text(""))
    });
    let r = r.inner.on_hover_text("Double-click to reset");
    r.widget_info(|| egui::WidgetInfo::slider(true, *v as f64, label));
    if r.double_clicked() {
        *v = default;
    }
    *v != before
}

/// A pill-shaped choice (aspect presets).
fn chip(ui: &mut Ui, label: &str, on: bool) -> bool {
    ui.add(
        egui::Button::new(RichText::new(label).size(house::BODY).color(if on { super::ACCENT } else { theme::TEXT }))
            .fill(if on { house::SELECTED } else { Color32::WHITE })
            .stroke(Stroke::new(1.0, if on { super::ACCENT } else { theme::BOX }))
            .corner_radius(16)
            .min_size(vec2(0.0, 32.0))
            .selected(on),
    )
    .clicked()
}

/// An icon button of the bottom bar.
fn icon_button(ui: &mut Ui, icon: &str, tip: &str, enabled: bool) -> bool {
    let b = match crate::icons::icon(icon) {
        Some(img) => egui::Button::image(img.fit_to_exact_size(vec2(20.0, 20.0))),
        None => egui::Button::new(tip),
    };
    let r = ui.add_enabled(enabled, b.frame(false).min_size(vec2(40.0, 40.0)));
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, tip));
    r.on_hover_text(tip).clicked()
}

fn to_color_image(img: &RgbaImage) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw())
}

/// Wall-clock milliseconds, for measuring a render.
fn now_ms() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64() * 1000.0)
    }
}

/// Decodes the picture (the browser on the web, a thread on the desktop) into the inbox.
fn decode(bytes: Vec<u8>, max: u32, inbox: Arc<Mutex<Vec<Got>>>, ctx: egui::Context) {
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(async move {
        let img = super::photos::decode_web(bytes, max).await;
        Editor::push(&inbox, &ctx, Got::Source(img));
    });
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(move || {
        let img = super::photos::decode_native(&bytes, max);
        Editor::push(&inbox, &ctx, Got::Source(img));
    });
}

// ------------------------------------------------------------ the viewer's side

/// One frame of the editor over the viewer: what it saved goes to the grid.
pub(super) fn ui(pm: &mut PhotoMode, ctx: &egui::Context, store: &Backend) {
    let Some(mut ed) = pm.editor.take() else { return };
    let out = ed.ui(ctx, store);
    if std::mem::take(&mut ed.relist)
        && let Some(f) = pm.folder.clone()
    {
        pm.list(ctx, store, &f);
    }
    match out {
        Outcome::Open => pm.editor = Some(ed),
        Outcome::Closed => {}
        Outcome::Saved(v) => pm.edited(&ed.id, v),
    }
}

/// Opens the editor on the picture the viewer shows.
pub(super) fn open(pm: &mut PhotoMode, ctx: &egui::Context, store: &Backend) {
    let Some(i) = pm.viewer.as_ref().map(|v| v.index()) else { return };
    let Some(p) = pm.pics.get(i) else { return };
    if matches!(p.state.as_str(), "failed" | "unsupported") {
        return;
    }
    pm.editor = Some(Editor::open(ctx, store, &p.id.clone(), &p.name.clone()));
}

#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code, reason = "the test hook is the web app's; tests use it too"))]
impl Editor {
    /// The test hook's view (app/TESTING.md).
    pub(super) fn test_state(&self) -> Json {
        json!({
            "open": true,
            "id": self.id,
            "tab": self.tab.name(),
            "edits": self.edits,
            "dirty": self.dirty(),
            "writable": self.writable,
            "whyNot": self.why_not,
            "foreign": self.foreign,
            "loaded": self.loaded,
            "source": self.src.as_ref().map(|s| [s.width(), s.height()]),
            "saving": self.saving,
            "ask": self.ask.as_ref().map(Ask::name),
            "error": self.error,
            "message": self.message,
            "comparing": self.comparing,
            "canUndo": self.can_undo(),
            "canRedo": self.can_redo(),
            "renderMs": (self.render_ms * 100.0).round() / 100.0,
            "renders": self.renders,
            "uploads": self.uploads,
            "preview": match &self.shown { None => "none", Some((_, true)) => "coarse", Some(_) => "full" },
        })
    }

    /// The test hook's commands (`home:photos:edit:<what>`): `tab {tab}`, `set {fields…}`,
    /// `slider {key, value}`, `undo`, `redo`, `reset`, `compare {on}`, `cancel`, `save`, `copy
    /// {strip_location, max_side}`, `answer {yes}`, `aspect {name}`, `rotate`, `flip`.
    pub(super) fn test_run(&mut self, ctx: &egui::Context, store: &Backend, what: &str, params: &Json) -> Result<(), String> {
        let s = |k: &str| params.get(k).and_then(Json::as_str).map(str::to_string);
        match what {
            "tab" => self.tab = Tab::from_name(&s("tab").unwrap_or_default()).ok_or("tab is crop, adjust or filters")?,
            "set" => self.set_json(params)?,
            "slider" => {
                let key = s("key").ok_or("slider needs {key, value}")?;
                let v = params.get("value").and_then(Json::as_f64).ok_or("slider needs {key, value}")? as f32;
                let k = knobs().into_iter().find(|k| k.key == key).ok_or_else(|| format!("no slider `{key}`"))?;
                (k.set)(&mut self.edits, v);
                self.settle(false);
            }
            "undo" => self.do_undo(),
            "redo" => self.do_redo(),
            "reset" => self.reset(),
            "compare" => self.compare_held = params.get("on").and_then(Json::as_bool).unwrap_or(true),
            "cancel" => self.cancel(),
            "save" => self.save(ctx, store),
            "copy" => {
                if let Some(b) = params.get("strip_location").and_then(Json::as_bool) {
                    self.strip_location = b;
                }
                self.max_side = params.get("max_side").and_then(Json::as_u64).map(|v| v as u32);
                self.copy(ctx, store);
            }
            "answer" => {
                let yes = params.get("yes").and_then(Json::as_bool).unwrap_or(true);
                if yes {
                    self.answer(ctx, store, true);
                } else {
                    self.ask = None;
                }
            }
            "aspect" => {
                self.set_aspect(&s("name").ok_or("aspect needs {name}")?);
                self.settle(false);
            }
            "rotate" => {
                self.rotate();
                self.settle(false);
            }
            "flip" => {
                self.flip();
                self.settle(false);
            }
            _ => return Err(format!("unknown editor command `{what}`")),
        }
        ctx.request_repaint();
        Ok(())
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests")]

    use std::cell::RefCell;
    use std::rc::Rc;

    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::store::{Answer, Done, Store};

    /// One call to the fake server: method, address, If-Match, body.
    type Call = (Method, String, Option<String>, Option<Json>);

    /// A server that answers from a script and remembers what it was asked.
    #[derive(Default)]
    struct Fake {
        calls: RefCell<Vec<Call>>,
        get: RefCell<Option<Answer>>,
        put: RefCell<Vec<Answer>>,
    }

    impl Store for Fake {
        fn call(&self, ctx: &egui::Context, method: Method, url: &str, body: Option<Vec<u8>>, done: Done) {
            self.call_if(ctx, method, url, body, None, done);
        }
        fn call_if(&self, _ctx: &egui::Context, method: Method, url: &str, body: Option<Vec<u8>>, if_match: Option<String>, done: Done) {
            let body = body.and_then(|b| serde_json::from_slice(&b).ok());
            self.calls.borrow_mut().push((method, url.to_string(), if_match, body));
            let a = match method {
                Method::Get if url.contains("/raw") => Ok((404, Vec::new())),
                Method::Get => self.get.borrow().clone().unwrap_or(Ok((404, Vec::new()))),
                Method::Put => self.put.borrow_mut().pop().unwrap_or(Ok((500, Vec::new()))),
                _ => Ok((200, br#"{"path":"ada/Pics/pic (edited).jpg","id":"x2"}"#.to_vec())),
            };
            done(a);
        }
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
        fn setting(&self, _: &str) -> Option<String> {
            None
        }
        fn set_setting(&self, _: &str, _: &str) {}
        fn has_photos(&self) -> bool {
            true
        }
    }

    struct St {
        ed: Editor,
        store: Backend,
        fake: Rc<Fake>,
        out: Vec<Outcome>,
    }

    fn reply(writable: bool, foreign: &[&str]) -> Answer {
        Ok((200, json!({ "edits": {"exposure": 0.5}, "etag": "e1", "writable": writable, "foreign": foreign }).to_string().into_bytes()))
    }

    fn harness(get: Answer) -> Harness<'static, St> {
        let fake = Rc::new(Fake::default());
        *fake.get.borrow_mut() = Some(get);
        let store = Backend(fake.clone());
        let ctx = egui::Context::default();
        let mut ed = Editor::open(&ctx, &store, "p1", "pic.jpg");
        ed.set_source(&egui::ColorImage::new([64, 48], vec![Color32::from_rgb(120, 90, 60); 64 * 48]));
        let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).build_ui_state(
            |ui, st: &mut St| {
                let o = st.ed.ui(ui.ctx(), &st.store);
                if o != Outcome::Open {
                    st.out.push(o);
                }
            },
            St { ed, store, fake, out: Vec::new() },
        );
        h.run();
        h
    }

    #[test]
    fn opens_with_the_saved_edits_and_closes_clean() {
        let mut h = harness(reply(true, &[]));
        let st = h.state();
        assert!(st.ed.loaded && st.ed.writable);
        assert_eq!(st.ed.edits.exposure, 0.5);
        assert!(!st.ed.dirty());
        assert_eq!(st.ed.test_state()["preview"], "full");
        let calls = st.fake.calls.borrow().iter().map(|c| c.1.clone()).collect::<Vec<_>>();
        assert_eq!(calls, vec!["api/photos/edits/p1", "api/files/p1/raw"]);
        h.get_by_label("Cancel").click();
        h.run();
        assert_eq!(h.state().out, vec![Outcome::Closed], "nothing changed: closes without asking");
    }

    #[test]
    fn a_slider_changes_the_edits_and_a_drag_is_one_undo_step() {
        let mut h = harness(reply(true, &[]));
        let c = egui::Context::default();
        let store = h.state().store.clone();
        h.state_mut().ed.test_run(&c, &store, "slider", &json!({"key": "contrast", "value": 30})).unwrap();
        h.state_mut().ed.test_run(&c, &store, "slider", &json!({"key": "temperature", "value": -20})).unwrap();
        h.run();
        let ed = &h.state().ed;
        assert_eq!((ed.edits.contrast, ed.edits.temperature), (30, -20));
        assert!(ed.dirty() && ed.can_undo());
        // A drag (pointer held) settles once, when let go.
        let ed = &mut h.state_mut().ed;
        ed.edits.shadows = 10;
        ed.settle(true);
        ed.edits.shadows = 40;
        ed.settle(true);
        ed.settle(false);
        assert_eq!(ed.undo.len(), 3, "two slider commands and one drag");
        ed.do_undo();
        assert_eq!(ed.edits.shadows, 0);
        assert!(ed.test_run(&c, &store, "slider", &json!({"key": "nope", "value": 1})).is_err());
    }

    #[test]
    fn undo_and_redo_keys_belong_to_the_editor() {
        let mut h = harness(reply(true, &[]));
        let c = egui::Context::default();
        let store = h.state().store.clone();
        h.state_mut().ed.test_run(&c, &store, "set", &json!({"saturation": 25, "rot90": 1})).unwrap();
        h.run();
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run();
        assert_eq!((h.state().ed.edits.saturation, h.state().ed.edits.rot90), (0, 0));
        assert!(h.state().ed.can_redo());
        h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
        h.run();
        assert_eq!(h.state().ed.edits.saturation, 25);
        // Keys never reach what is behind the editor.
        h.key_press(Key::ArrowRight);
        h.run();
        assert!(h.ctx.input(|i| i.events.iter().all(|e| !matches!(e, egui::Event::Key { .. }))));
        // Hold \ to compare.
        h.key_down(Key::Backslash);
        h.run();
        assert!(h.state().ed.comparing);
        h.key_up(Key::Backslash);
        h.run();
        assert!(!h.state().ed.comparing);
    }

    #[test]
    fn sliders_share_one_scale_and_a_double_click_resets() {
        let mut h = harness(reply(true, &[]));
        let c = egui::Context::default();
        let store = h.state().store.clone();
        h.state_mut().ed.test_run(&c, &store, "slider", &json!({"key": "exposure", "value": 0.8})).unwrap();
        h.state_mut().ed.test_run(&c, &store, "slider", &json!({"key": "contrast", "value": 40})).unwrap();
        h.run();
        assert_eq!(shown(true, 0.8), "+16");
        assert_eq!(shown(true, -5.0), "-100");
        assert_eq!(shown(false, 40.0), "+40");
        assert!(h.query_by_label("+0.80").is_none(), "no stops shown");
        assert!(h.query_by_label("+16").is_some());
        let s = h.get_all_by_role(egui::accesskit::Role::Slider).nth(1).unwrap();
        s.click();
        s.click();
        h.run();
        assert_eq!(h.state().ed.edits.contrast, 0, "double click resets");
    }

    #[test]
    fn cancel_with_changes_asks_first() {
        let mut h = harness(reply(true, &[]));
        let c = egui::Context::default();
        let store = h.state().store.clone();
        h.state_mut().ed.test_run(&c, &store, "slider", &json!({"key": "exposure", "value": 1.0})).unwrap();
        h.run();
        h.get_by_label("Cancel").click();
        h.run();
        assert_eq!(h.state().ed.ask, Some(Ask::Discard));
        assert!(h.state().out.is_empty());
        h.get_by_label("Keep editing").click();
        h.run();
        assert_eq!(h.state().ed.ask, None);
        h.key_press(Key::Escape);
        h.run();
        assert_eq!(h.state().ed.ask, Some(Ask::Discard), "Esc is Cancel too");
        h.get_by_label("Discard").click();
        h.run();
        assert_eq!(h.state().out, vec![Outcome::Closed]);
    }

    #[test]
    fn save_sends_if_match_and_a_conflict_offers_reload() {
        let mut h = harness(reply(true, &[]));
        let c = egui::Context::default();
        let store = h.state().store.clone();
        h.state().fake.put.borrow_mut().push(Ok((409, br#"{"error":"changed"}"#.to_vec())));
        h.state_mut().ed.test_run(&c, &store, "slider", &json!({"key": "vibrance", "value": 15})).unwrap();
        h.run();
        h.get_by_label("Save").click();
        h.run();
        h.run();
        {
            let calls = h.state().fake.calls.borrow();
            let put = calls.iter().find(|c| c.0 == Method::Put).unwrap();
            assert_eq!(put.1, "api/photos/edits/p1");
            assert_eq!(put.2.as_deref(), Some("e1"));
            assert_eq!(put.3.as_ref().unwrap()["vibrance"], 15);
        }
        assert_eq!(h.state().ed.ask, Some(Ask::Conflict));
        assert!(h.state().out.is_empty() && h.state().ed.dirty());
        h.get_by_label("Reload").click();
        h.run();
        h.run();
        assert_eq!(h.state().ed.edits.vibrance, 0, "their edits are back");
        assert!(!h.state().ed.dirty());
        // Saving again goes through and closes, with the new thumbnail version.
        h.state().fake.put.borrow_mut().push(Ok((200, br#"{"etag":"e2","thumb_v":"t2"}"#.to_vec())));
        h.state_mut().ed.test_run(&c, &store, "slider", &json!({"key": "tint", "value": 5})).unwrap();
        h.state_mut().ed.test_run(&c, &store, "save", &json!({})).unwrap();
        h.run();
        assert_eq!(h.state().out, vec![Outcome::Saved(Some("t2".into()))]);
    }

    #[test]
    fn read_only_pictures_cannot_be_saved_and_say_why() {
        let mut h = harness(reply(false, &[]));
        let c = egui::Context::default();
        let store = h.state().store.clone();
        h.state_mut().ed.test_run(&c, &store, "slider", &json!({"key": "contrast", "value": 10})).unwrap();
        h.run();
        assert!(!h.state().ed.writable);
        h.get_by_label_contains("can't be changed");
        h.state_mut().ed.test_run(&c, &store, "save", &json!({})).unwrap();
        h.run();
        assert!(h.state().fake.calls.borrow().iter().all(|c| c.0 != Method::Put), "never sent");
        // No server support at all: the same, with its own reason.
        let h = harness(Ok((404, Vec::new())));
        assert!(!h.state().ed.writable && h.state().ed.why_not.as_deref().unwrap_or("").contains("can't keep"));
    }

    #[test]
    fn save_a_copy_sends_the_edits_and_never_saves_the_original() {
        let mut h = harness(reply(true, &[]));
        let c = egui::Context::default();
        let store = h.state().store.clone();
        h.state_mut().ed.test_run(&c, &store, "slider", &json!({"key": "contrast", "value": 30})).unwrap();
        h.state_mut().ed.test_run(&c, &store, "copy", &json!({})).unwrap();
        h.run();
        let calls = h.state().fake.calls.borrow().clone();
        assert!(calls.iter().all(|c| c.0 != Method::Put), "the original's edits are not saved");
        let post = calls.iter().find(|c| c.0 == Method::Post).expect("export asked");
        assert_eq!(post.1, "api/photos/export/p1");
        let body = post.3.clone().unwrap();
        assert_eq!(body["edits"]["contrast"], json!(h.state().ed.edits.contrast), "{body}");
        assert_eq!(body["edits"]["exposure"], 0.5);
    }

    #[test]
    fn darktable_edits_are_mentioned_once_before_the_first_save() {
        let mut h = harness(reply(true, &["darktable"]));
        let c = egui::Context::default();
        let store = h.state().store.clone();
        h.state().fake.put.borrow_mut().push(Ok((200, br#"{"etag":"e2","thumb_v":"t2"}"#.to_vec())));
        h.state_mut().ed.test_run(&c, &store, "slider", &json!({"key": "contrast", "value": 10})).unwrap();
        h.state_mut().ed.test_run(&c, &store, "save", &json!({})).unwrap();
        h.run();
        assert_eq!(h.state().ed.ask, Some(Ask::Foreign));
        h.get_by_label_contains("darktable");
        h.get_by_label("Save anyway").click();
        h.run();
        h.run();
        assert_eq!(h.state().out, vec![Outcome::Saved(Some("t2".into()))]);
    }

    #[test]
    fn tabs_and_the_crop_tools() {
        let mut h = harness(reply(true, &[]));
        h.get_by_label("Filters").click();
        h.run();
        assert_eq!(h.state().ed.tab, Tab::Filters);
        assert_eq!(h.state().ed.filter_tex.len(), 6);
        h.get_by_label("Sepia").click();
        h.run();
        assert_eq!(h.state().ed.edits.filter.map(|f| f.name), Some(FilterName::Sepia));
        h.get_by_label("Crop").click();
        h.run();
        h.get_by_label("Square").click();
        h.run();
        let c = h.state().ed.edits.crop;
        // 64 × 48: a square is 0.75 of the width, all of the height, in the middle.
        assert!((c.r - c.l - 0.75).abs() < 1e-4 && (c.b - c.t - 1.0).abs() < 1e-4 && (c.l - 0.125).abs() < 1e-4, "{c:?}");
        h.get_by_label("Rotate 90°").click();
        h.run();
        assert_eq!(h.state().ed.edits.rot90, 3, "a quarter to the left");
        h.get_by_label("Flip").click();
        h.run();
        assert!(h.state().ed.edits.flip_h);
        h.get_by_label("Reset").click();
        h.run();
        assert!(h.state().ed.edits.is_identity());
        let renders = h.state().ed.renders;
        h.run();
        h.run();
        assert_eq!(h.state().ed.renders, renders, "nothing rendered again when nothing changed");
    }

    #[test]
    fn crop_maths() {
        let full = Crop::default();
        let sq = fit_aspect(full, 1.0, 2.0);
        assert!((sq.r - sq.l - 0.5).abs() < 1e-6 && (sq.b - sq.t - 1.0).abs() < 1e-6);
        let moved = drag_crop(sq, Grab::Move, 0.9, 0.0, Some(1.0), 2.0);
        assert!((moved.r - 1.0).abs() < 1e-6, "stays inside");
        let smaller = drag_crop(sq, Grab::Edges(false, false, true, false), -0.1, 0.0, Some(1.0), 2.0);
        assert!(((smaller.r - smaller.l) * 2.0 - (smaller.b - smaller.t)).abs() < 1e-5, "keeps square: {smaller:?}");
        let free = drag_crop(full, Grab::Edges(true, true, false, false), 0.2, 0.3, None, 2.0);
        assert_eq!((free.l, free.t, free.r, free.b), (0.2, 0.3, 1.0, 1.0));
        let tiny = drag_crop(full, Grab::Edges(true, false, false, false), 2.0, 0.0, None, 1.0);
        assert!(tiny.r - tiny.l >= MIN_CROP - 1e-6);
    }
}
