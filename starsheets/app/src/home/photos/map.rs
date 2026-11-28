//! Photos → Places as a world map: a built-in offline base map (Natural Earth, public domain,
//! built by scripts/worldmap.py into assets/worldmap.bin), optional detailed tiles fetched through
//! the server (`STARBASE_MAP_TILES`), and the pictures stacked where they were taken. Clicking a
//! stack opens it in a strip along the bottom of the map; a picture there opens the viewer.

// The stack and strip rects are kept for the web test hook only.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use std::collections::HashMap;
use std::sync::OnceLock;

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2, pos2, vec2};
use serde_json::Value as Json;
#[cfg(target_arch = "wasm32")]
use serde_json::json;

use super::{Got, PhotoMode, Pic, SMALL, Viewer};
use crate::house;
use crate::store::Backend;
use crate::theme;

/// The map's own palette: dark, so Photo Mode's dark theme carries on into it.
const SEA: Color32 = Color32::from_rgb(0x17, 0x1A, 0x1F);
const LAND: Color32 = Color32::from_rgb(0x2B, 0x30, 0x37);
const COAST: Color32 = Color32::from_rgb(0x5F, 0x66, 0x70);
const BORDER: Color32 = Color32::from_rgb(0x4A, 0x50, 0x58);
const LABEL: Color32 = Color32::from_rgb(0x9A, 0xA0, 0xA6);
/// Stacks closer than this on screen (points) become one.
const CLUSTER: f32 = 72.0;
/// A stack's cover side.
const THUMB: f32 = 56.0;
const MIN_ZOOM: f32 = 0.0;
const MAX_ZOOM: f32 = 18.0;
/// The offline preview needs a wide enough view for the bundled map's regional detail.
const MINI_OVERVIEW_ZOOM: f32 = 6.0;
const MINI_TILE_ZOOM: f32 = 11.0;
/// The Places map's zoom when opened from the small map.
const PLACES_ZOOM: f32 = 13.0;
/// Past this zoom the built-in map shows its close-up band (1:50m).
const CLOSE_UP: f32 = 2.6;
/// Detailed tiles (when the server has them) take over from this zoom.
const TILES_FROM: f32 = 3.0;

const WORLD: &[u8] = include_bytes!("../../../assets/worldmap.bin");

/// One decoded layer: lines in Web Mercator (0..1 both ways).
struct Layer {
    kind: u8,
    band: u8,
    items: Vec<Vec<[f32; 2]>>,
}

/// A label: a place name at a position.
struct Label {
    at: [f32; 2],
    rank: u8,
    name: String,
}

/// A filled layer, triangulated once: vertices, triangles, and each polygon's box and range.
#[derive(Default)]
struct Fill {
    pts: Vec<[f32; 2]>,
    tris: Vec<u32>,
    /// (box min, box max, first triangle index, end)
    parts: Vec<([f32; 2], [f32; 2], usize, usize)>,
}

struct World {
    layers: Vec<Layer>,
    labels: Vec<Label>,
    /// Land and lakes for bands 0 and 1: [band][0 land, 1 lake].
    fills: [[OnceLock<Fill>; 2]; 2],
}

fn varint(b: &[u8], at: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let c = *b.get(*at)?;
        *at += 1;
        v |= u64::from(c & 0x7F) << shift;
        if c & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

fn unzig(n: u64) -> i64 {
    ((n >> 1) as i64) ^ -((n & 1) as i64)
}

const Q: f32 = (1u32 << 20) as f32;

/// Reads the "SBMAP1" map (format: scripts/worldmap.py).
fn decode(b: &[u8]) -> Option<(Vec<Layer>, Vec<Label>)> {
    let rest = b.strip_prefix(b"SBMAP1")?;
    let mut at = 1;
    let (mut layers, mut labels) = (Vec::new(), Vec::new());
    for _ in 0..*rest.first()? {
        let (kind, band) = (*rest.get(at)?, *rest.get(at + 1)?);
        at += 2;
        let n = varint(rest, &mut at)?;
        let mut items = Vec::new();
        for _ in 0..n {
            if kind == 3 {
                let x = varint(rest, &mut at)? as f32 / Q;
                let y = varint(rest, &mut at)? as f32 / Q;
                let rank = *rest.get(at)?;
                at += 1;
                let len = varint(rest, &mut at)? as usize;
                let name = String::from_utf8_lossy(rest.get(at..at + len)?).into_owned();
                at += len;
                labels.push(Label { at: [x, y], rank, name });
                continue;
            }
            let m = varint(rest, &mut at)?.min(1 << 20) as usize;
            let (mut x, mut y) = (0i64, 0i64);
            let mut pts = Vec::with_capacity(m);
            for _ in 0..m {
                x += unzig(varint(rest, &mut at)?);
                y += unzig(varint(rest, &mut at)?);
                pts.push([x as f32 / Q, y as f32 / Q]);
            }
            items.push(pts);
        }
        if kind != 3 {
            layers.push(Layer { kind, band, items });
        }
    }
    Some((layers, labels))
}

fn world() -> &'static World {
    static W: OnceLock<World> = OnceLock::new();
    W.get_or_init(|| {
        let (layers, labels) = decode(WORLD).unwrap_or_default();
        World { layers, labels, fills: Default::default() }
    })
}

impl World {
    fn fill(&self, band: u8, kind: u8) -> &Fill {
        self.fills[band as usize][kind as usize].get_or_init(|| {
            let mut f = Fill::default();
            for l in self.layers.iter().filter(|l| l.band == band && l.kind == kind) {
                for ring in &l.items {
                    let base = f.pts.len() as u32;
                    let first = f.tris.len();
                    f.tris.extend(triangulate(ring).into_iter().map(|i| base + i));
                    f.pts.extend_from_slice(ring);
                    let (lo, hi) = bounds(ring);
                    f.parts.push((lo, hi, first, f.tris.len()));
                }
            }
            f
        })
    }
}

fn bounds(pts: &[[f32; 2]]) -> ([f32; 2], [f32; 2]) {
    pts.iter().fold(([f32::MAX; 2], [f32::MIN; 2]), |(lo, hi), p| ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]))
}

fn cross(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Ear clipping of one ring (closed or not): triangle corner indexes into `ring`. Rings that
/// simplification made slightly self-touching still finish (a stuck corner is cut anyway).
pub(super) fn triangulate(ring: &[[f32; 2]]) -> Vec<u32> {
    let mut n = ring.len();
    if n > 1 && ring[0] == ring[n - 1] {
        n -= 1;
    }
    if n < 3 {
        return Vec::new();
    }
    let area: f32 = (0..n).map(|i| cross([0.0, 0.0], ring[i], ring[(i + 1) % n])).sum();
    let mut idx: Vec<u32> = (0..n as u32).collect();
    if area < 0.0 {
        idx.reverse();
    }
    let p = |i: u32| ring[i as usize];
    let mut out = Vec::with_capacity(3 * n);
    let (mut i, mut tries) = (0usize, 0usize);
    while idx.len() > 3 {
        let m = idx.len();
        let (a, b, c) = (idx[(i + m - 1) % m], idx[i % m], idx[(i + 1) % m]);
        let (pa, pb, pc) = (p(a), p(b), p(c));
        let convex = cross(pa, pb, pc) > 0.0;
        let ear = convex && {
            let lo = [pa[0].min(pb[0]).min(pc[0]), pa[1].min(pb[1]).min(pc[1])];
            let hi = [pa[0].max(pb[0]).max(pc[0]), pa[1].max(pb[1]).max(pc[1])];
            !idx.iter().any(|&k| {
                let q = p(k);
                k != a
                    && k != b
                    && k != c
                    && q[0] >= lo[0]
                    && q[0] <= hi[0]
                    && q[1] >= lo[1]
                    && q[1] <= hi[1]
                    && cross(pa, pb, q) >= 0.0
                    && cross(pb, pc, q) >= 0.0
                    && cross(pc, pa, q) >= 0.0
            })
        };
        if ear || tries > m {
            out.extend([a, b, c]);
            idx.remove(i % m);
            tries = 0;
            i = i.saturating_sub(1);
        } else {
            i = (i + 1) % m;
            tries += 1;
        }
    }
    out.extend(idx);
    out
}

/// Pictures stacked by screen distance at a zoom: (stack centre in world points, picture indexes).
/// A grid of `cell` points: pictures in one cell share a stack; the stack sits at their middle.
pub(super) fn cluster(points: &[[f32; 2]], world_px: f32, cell: f32) -> Vec<([f32; 2], Vec<usize>)> {
    let mut cells: HashMap<(i64, i64), usize> = HashMap::new();
    let mut out: Vec<([f32; 2], Vec<usize>)> = Vec::new();
    for (i, p) in points.iter().enumerate() {
        let (x, y) = (p[0] * world_px, p[1] * world_px);
        let key = ((x / cell).floor() as i64, (y / cell).floor() as i64);
        let at = *cells.entry(key).or_insert_with(|| {
            out.push(([0.0, 0.0], Vec::new()));
            out.len() - 1
        });
        out[at].0[0] += x;
        out[at].0[1] += y;
        out[at].1.push(i);
    }
    for s in &mut out {
        let n = s.1.len() as f32;
        s.0 = [s.0[0] / n, s.0[1] / n];
    }
    out
}

/// Latitude/longitude to Web Mercator (0..1 both ways, y down).
pub(super) fn mercator(lat: f64, lon: f64) -> [f32; 2] {
    let lat = lat.clamp(-85.051_128, 85.051_128).to_radians();
    let x = (lon + 180.0) / 360.0;
    let y = 0.5 - (lat.tan() + 1.0 / lat.cos()).ln() / (2.0 * std::f64::consts::PI);
    [x.clamp(0.0, 1.0) as f32, y.clamp(0.0, 1.0) as f32]
}

#[derive(Default)]
pub(in crate::home) struct PlacesMap {
    /// The cards instead of the map.
    pub list: bool,
    asked: bool,
    pub loaded: bool,
    /// Every picture with a position; `pics` is this through the grid's Type filter and search.
    all: Vec<Pic>,
    sig: String,
    pics: Vec<Pic>,
    pts: Vec<[f32; 2]>,
    /// The server's detailed tiles: (deepest zoom, credit line).
    tiles: Option<(u32, String)>,
    /// The middle of the view (Mercator) and the zoom (world = 256 × 2^zoom points wide).
    centre: [f32; 2],
    zoom: f32,
    fitted: bool,
    /// The stack open in the bottom strip (picture indexes).
    open: Option<Vec<usize>>,
    /// Stacks drawn last frame, for the test hook: (screen rect, picture indexes).
    drawn: Vec<(Rect, Vec<usize>)>,
    /// Thumbnails in the open strip last frame (screen rects).
    strip: Vec<Rect>,
    view: Option<Rect>,
}

impl PlacesMap {
    pub fn take(&mut self, v: &Json) {
        self.all = PhotoMode::parse_pics(v);
        self.all.retain(|p| p.gps.is_some());
        self.sig = String::from("\u{0}");
        self.tiles = v["tiles"]["max_z"].as_u64().map(|z| (z as u32, v["tiles"]["attribution"].as_str().unwrap_or("").to_string()));
        self.loaded = true;
        self.fitted = false;
        self.open = None;
    }

    /// Fit every picture into `view` (the whole world when there are none).
    fn fit(&mut self, view: Rect) {
        if self.pts.is_empty() {
            self.centre = [0.5, 0.5];
            self.zoom = (view.width().min(view.height() * 1.4) / 256.0).log2().clamp(MIN_ZOOM, MAX_ZOOM);
            return;
        }
        let (lo, hi) = bounds(&self.pts);
        self.centre = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
        let span = (hi[0] - lo[0]).max(1e-5).max((hi[1] - lo[1]) * view.width() / view.height().max(1.0));
        self.zoom = ((view.width() * 0.7 / span) / 256.0).log2().clamp(MIN_ZOOM, 14.0);
    }

    fn world_px(&self) -> f32 {
        256.0 * self.zoom.exp2()
    }

    fn to_screen(&self, view: Rect, m: [f32; 2], shift: f32) -> Pos2 {
        let w = self.world_px();
        pos2(view.center().x + (m[0] + shift - self.centre[0]) * w, view.center().y + (m[1] - self.centre[1]) * w)
    }

    /// Zoom by `steps` keeping the world point under `at` still.
    fn zoom_at(&mut self, view: Rect, at: Pos2, steps: f32) {
        let w0 = self.world_px();
        let before = [self.centre[0] + (at.x - view.center().x) / w0, self.centre[1] + (at.y - view.center().y) / w0];
        self.zoom = (self.zoom + steps).clamp(MIN_ZOOM, MAX_ZOOM);
        let w1 = self.world_px();
        self.centre = [before[0] - (at.x - view.center().x) / w1, before[1] - (at.y - view.center().y) / w1];
        self.clamp(view);
    }

    fn clamp(&mut self, view: Rect) {
        let w = self.world_px();
        self.centre[0] = self.centre[0].rem_euclid(1.0);
        let half = (view.height() / 2.0 / w).min(0.5);
        self.centre[1] = self.centre[1].clamp(half, 1.0 - half);
    }

    #[cfg(target_arch = "wasm32")]
    pub fn test_state(&self) -> Json {
        let r = |r: &Rect| json!([r.min.x, r.min.y, r.width(), r.height()]);
        json!({
            "list": self.list, "loaded": self.loaded, "zoom": self.zoom, "centre": self.centre, "pictures": self.pics.len(),
            "tiles": self.tiles.as_ref().map(|t| &t.1), "view": self.view.as_ref().map(r),
            "stacks": self.drawn.iter().map(|(rc, ix)| json!({"rect": r(rc), "count": ix.len()})).collect::<Vec<_>>(),
            "open": self.open.as_ref().map(Vec::len), "strip": self.strip.iter().map(r).collect::<Vec<_>>(),
        })
    }
}

impl PhotoMode {
    /// Asks the server for every picture with a position (once per visit).
    pub(super) fn map_boot(&mut self, ctx: &egui::Context, store: &Backend) {
        if !self.map.asked {
            self.map.asked = true;
            super::albums::get(ctx, store, &self.library_inbox, "api/photos/places?all=1", Got::Map);
        }
    }

    /// The Info panel's small map around (`lat`, `lon`) with a pin: the built-in map, and the
    /// server's tiles once the Places map knows them. Returns whether it was clicked.
    pub(in crate::home) fn mini_map(&mut self, ui: &Ui, store: &Backend, view: Rect, lat: f64, lon: f64) -> bool {
        let ctx = ui.ctx().clone();
        self.map_boot(&ctx, store);
        let saved = (self.map.centre, self.map.zoom);
        self.map.centre = mercator(lat, lon);
        self.map.zoom = if self.map.tiles.is_some() { MINI_TILE_ZOOM } else { MINI_OVERVIEW_ZOOM };
        let painter = ui.painter_at(view);
        painter.rect_filled(view, 4, SEA);
        self.draw_base(&painter, view);
        self.draw_tiles(&ctx, store, &painter, view);
        (self.map.centre, self.map.zoom) = saved;
        let pin = view.center();
        painter.circle_filled(pin, 7.0, Color32::from_rgb(0xEA, 0x43, 0x35));
        painter.circle_stroke(pin, 7.0, Stroke::new(2.0, Color32::WHITE));
        house::target(&ctx, "viewer:info:map", view);
        ui.interact(view, egui::Id::unique("info_mini_map"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
    }

    /// The Places map (not its list), centred close in on (`lat`, `lon`).
    pub(in crate::home) fn places_at(&mut self, lat: f64, lon: f64) {
        self.map.list = false;
        self.map.centre = mercator(lat, lon);
        self.map.zoom = PLACES_ZOOM;
        self.map.fitted = true;
        self.map.open = None;
    }

    /// The Places map, filling `ui`.
    pub(super) fn map_ui(&mut self, ui: &mut Ui, store: &Backend) {
        let ctx = ui.ctx().clone();
        self.map_boot(&ctx, store);
        let sig = format!("{:?}|{}", self.shell.types, self.shell.query.trim().to_lowercase());
        if self.map.sig != sig && self.map.loaded {
            let pics: Vec<Pic> = self.map.all.iter().filter(|p| self.shell.matches(p)).cloned().collect();
            self.map.pts = pics.iter().filter_map(|p| p.gps).map(|(lat, lon)| mercator(lat, lon)).collect();
            self.map.pics = pics;
            self.map.sig = sig;
            self.map.fitted = false;
            self.map.open = None;
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new("Places").size(house::TITLE).color(theme::text()));
            if self.pill(ui, "List", false) {
                self.map.list = true;
            }
            if self.pill(ui, "Fit all", false) {
                self.map.fitted = false;
            }
            if self.pill(ui, "−", false) {
                let v = self.map.view.unwrap_or(Rect::ZERO);
                self.map.zoom_at(v, v.center(), -1.0);
            }
            if self.pill(ui, "+", false) {
                let v = self.map.view.unwrap_or(Rect::ZERO);
                self.map.zoom_at(v, v.center(), 1.0);
            }
            if self.map.loaded && self.map.pics.is_empty() {
                ui.label(RichText::new("No pictures with a position (GPS) yet.").size(house::NOTE).color(theme::text_dim()));
            }
        });
        ui.add_space(house::S);
        let size = ui.available_size().max(vec2(120.0, 160.0));
        let (view, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        self.map.view = Some(view);
        if !self.map.fitted && view.width() > 1.0 {
            self.map.fit(view);
            self.map.fitted = true;
        }
        let strip_h = if self.map.open.is_some() { view.height() / 2.0 } else { 0.0 };
        let strip = Rect::from_min_max(pos2(view.min.x, view.max.y - strip_h), view.max);
        self.map_input(ui, view, strip, &resp);
        let painter = ui.painter_at(view);
        painter.rect_filled(view, house::PANEL_RADIUS, SEA);
        self.draw_base(&painter, view);
        self.draw_tiles(&ctx, store, &painter, view);
        self.draw_stacks(ui, &ctx, store, view, strip_h, &resp);
        if let Some((_, credit)) = &self.map.tiles
            && self.map.zoom >= TILES_FROM
        {
            painter.text(view.max - vec2(6.0, strip_h + 4.0), egui::Align2::RIGHT_BOTTOM, credit, egui::FontId::proportional(house::CAPS), LABEL);
        }
        self.map.strip.clear();
        if self.map.open.is_some() {
            self.strip_ui(ui, &ctx, store, strip);
        }
    }

    fn map_input(&mut self, ui: &Ui, view: Rect, strip: Rect, resp: &egui::Response) {
        let m = &mut self.map;
        let over_map = resp.hover_pos().is_some_and(|p| !strip.contains(p) || strip.height() == 0.0);
        if resp.dragged() && resp.interact_pointer_pos().is_some_and(|p| !strip.contains(p) || strip.height() == 0.0) {
            let w = m.world_px();
            let d = resp.drag_delta();
            m.centre = [m.centre[0] - d.x / w, m.centre[1] - d.y / w];
            m.clamp(view);
        }
        if over_map {
            let at = resp.hover_pos().unwrap_or(view.center());
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            if scroll != 0.0 {
                m.zoom_at(view, at, (scroll / 200.0).clamp(-1.0, 1.0));
            }
            if pinch != 1.0 {
                m.zoom_at(view, at, pinch.log2());
            }
            if resp.double_clicked() {
                m.zoom_at(view, at, 1.0);
            }
        }
        if resp.hovered() || resp.has_focus() {
            let step = 60.0 / m.world_px();
            ui.input(|i| {
                use egui::Key::*;
                let dx = f32::from(i.key_down(ArrowRight)) - f32::from(i.key_down(ArrowLeft));
                let dy = f32::from(i.key_down(ArrowDown)) - f32::from(i.key_down(ArrowUp));
                m.centre = [m.centre[0] + dx * step * 0.2, m.centre[1] + dy * step * 0.2];
            });
            m.clamp(view);
        }
        if m.open.is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) && self.viewer.is_none() {
            m.open = None;
        }
    }

    /// The copies of the world across the view (the map wraps east–west).
    fn shifts(&self, view: Rect) -> Vec<f32> {
        let m = &self.map;
        let w = m.world_px();
        let left = m.centre[0] - view.width() / 2.0 / w;
        let right = m.centre[0] + view.width() / 2.0 / w;
        (left.floor() as i32..=right.floor() as i32).map(|k| k as f32).collect()
    }

    fn draw_base(&self, painter: &egui::Painter, view: Rect) {
        let m = &self.map;
        let wd = world();
        let band = u8::from(m.zoom >= CLOSE_UP);
        let w = m.world_px();
        for shift in self.shifts(view) {
            for (kind, colour) in [(0u8, LAND), (1, SEA)] {
                let f = wd.fill(band, kind);
                let mut mesh = Mesh::default();
                let mut map: HashMap<u32, u32> = HashMap::new();
                for (lo, hi, a, b) in &f.parts {
                    let r = Rect::from_min_max(m.to_screen(view, *lo, shift), m.to_screen(view, *hi, shift));
                    if !r.intersects(view) || (r.width() < 0.6 && r.height() < 0.6) {
                        continue;
                    }
                    for &i in &f.tris[*a..*b] {
                        let v = *map.entry(i).or_insert_with(|| {
                            mesh.vertices.push(Vertex { pos: m.to_screen(view, f.pts[i as usize], shift), uv: WHITE_UV, color: colour });
                            mesh.vertices.len() as u32 - 1
                        });
                        mesh.indices.push(v);
                    }
                }
                painter.add(mesh);
            }
            let stroke = |c: Color32| Stroke::new(if w > 3000.0 { 1.0 } else { 0.7 }, c);
            for l in wd.layers.iter().filter(|l| l.band == band) {
                let s = match l.kind {
                    0 => stroke(COAST),
                    2 => stroke(BORDER),
                    _ => continue,
                };
                for line in &l.items {
                    let (lo, hi) = bounds(line);
                    let r = Rect::from_min_max(m.to_screen(view, lo, shift), m.to_screen(view, hi, shift));
                    if !r.intersects(view) || (r.width() < 1.5 && r.height() < 1.5) {
                        continue;
                    }
                    painter.add(egui::Shape::line(line.iter().map(|p| m.to_screen(view, *p, shift)).collect(), s));
                }
            }
            // Town names, more of them as you zoom in.
            let most = (m.zoom * 2.5) as u8;
            for l in wd.labels.iter().filter(|l| l.rank <= most) {
                let at = m.to_screen(view, l.at, shift);
                if view.contains(at) {
                    painter.circle_filled(at, 2.0, LABEL);
                    painter.text(at + vec2(4.0, 0.0), egui::Align2::LEFT_CENTER, &l.name, egui::FontId::proportional(house::CAPS), LABEL);
                }
            }
        }
    }

    /// The server's detailed tiles over the built-in map, when there are any.
    fn draw_tiles(&mut self, ctx: &egui::Context, store: &Backend, painter: &egui::Painter, view: Rect) {
        let Some((max_z, _)) = self.map.tiles.clone() else { return };
        if self.map.zoom < TILES_FROM {
            return;
        }
        let z = (self.map.zoom.round() as u32).min(max_z);
        let n = 1u32 << z;
        let tile = self.map.world_px() / n as f32;
        let (c, half) = (self.map.centre, vec2(view.width(), view.height()) / 2.0 / self.map.world_px());
        let (x0, x1) = (((c[0] - half.x) * n as f32).floor() as i64, ((c[0] + half.x) * n as f32).floor() as i64);
        let (y0, y1) = ((((c[1] - half.y) * n as f32).floor() as i64).max(0), (((c[1] + half.y) * n as f32).floor() as i64).min(n as i64 - 1));
        for ty in y0..=y1 {
            for tx in x0..=x1 {
                let id = format!("tile/{z}/{}/{ty}", tx.rem_euclid(n as i64));
                self.want_id(ctx, store, &id, None, SMALL);
                if let Some(t) = self.tex.get(&(id, SMALL), self.frame) {
                    let min = self.map.to_screen(view, [tx as f32 / n as f32, ty as f32 / n as f32], 0.0);
                    let r = Rect::from_min_size(min, Vec2::splat(tile));
                    painter.image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
            }
        }
    }

    fn draw_stacks(&mut self, ui: &Ui, ctx: &egui::Context, store: &Backend, view: Rect, strip_h: f32, resp: &egui::Response) {
        let w = self.map.world_px();
        let stacks = cluster(&self.map.pts, w, CLUSTER);
        let painter = ui.painter_at(Rect::from_min_max(view.min, pos2(view.max.x, view.max.y - strip_h)));
        self.map.drawn.clear();
        let mut clicked = None;
        let click = resp.clicked().then(|| resp.interact_pointer_pos()).flatten();
        for shift in self.shifts(view) {
            for (at, ix) in &stacks {
                let p = self.map.to_screen(view, [at[0] / w, at[1] / w], shift);
                let r = Rect::from_center_size(p, Vec2::splat(THUMB));
                if !r.intersects(view) {
                    continue;
                }
                let cover = &self.map.pics[ix[0]];
                let (id, v) = (cover.id.clone(), cover.v.clone());
                painter.rect_filled(r.expand(2.0), 6, Color32::WHITE);
                painter.rect_filled(r, 4, LAND);
                self.want_id(ctx, store, &id, v.as_deref(), SMALL);
                if let Some(t) = self.tex.get(&(id, SMALL), self.frame) {
                    let [tw, th] = t.size();
                    painter.image(t.id(), r, super::cover_uv(tw as f32, th as f32), Color32::WHITE);
                }
                if ix.len() > 1 {
                    let b = r.right_top() + vec2(-2.0, 2.0);
                    let text = ix.len().to_string();
                    let rad = 9.0 + 3.0 * (text.len() as f32 - 1.0);
                    painter.circle_filled(b, rad, house::BLUE);
                    painter.text(b, egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(house::CAPS), Color32::WHITE);
                }
                if click.is_some_and(|c| r.contains(c) && c.y < view.max.y - strip_h) {
                    clicked = Some(ix.clone());
                }
                self.map.drawn.push((r, ix.clone()));
            }
        }
        if let Some(ix) = clicked {
            self.map.open = Some(ix);
        }
    }

    /// The open stack: a strip along the bottom, the map's width and half its height.
    fn strip_ui(&mut self, ui: &mut Ui, ctx: &egui::Context, store: &Backend, strip: Rect) {
        let Some(ix) = self.map.open.clone() else { return };
        let painter = ui.painter_at(strip);
        painter.rect_filled(strip, 0, Color32::from_rgba_unmultiplied(0x10, 0x11, 0x14, 240));
        let head = Rect::from_min_size(strip.min, vec2(strip.width(), 36.0));
        let n = ix.len();
        let place = self.map.pics[ix[0]].place.clone().unwrap_or_default();
        painter.text(
            head.left_center() + vec2(house::M, 0.0),
            egui::Align2::LEFT_CENTER,
            format!("{place}{}{n} {}", if place.is_empty() { "" } else { " · " }, if n == 1 { "picture" } else { "pictures" }),
            egui::FontId::proportional(house::BODY),
            Color32::WHITE,
        );
        let close = Rect::from_center_size(head.right_center() - vec2(22.0, 0.0), Vec2::splat(28.0));
        let cr = ui.interact(close, ui.scope_id().with("map-strip-close"), Sense::click());
        painter.text(close.center(), egui::Align2::CENTER_CENTER, "×", egui::FontId::proportional(22.0), Color32::WHITE);
        if cr.clicked() {
            self.map.open = None;
            return;
        }
        let body = Rect::from_min_max(pos2(strip.min.x + house::S, head.max.y), strip.max - vec2(house::S, house::S));
        let h = body.height().max(40.0);
        let mut open = None;
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
        egui::ScrollArea::horizontal().id_salt("map-strip").auto_shrink([false, false]).show(&mut child, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(house::S, 0.0);
                for (k, &i) in ix.iter().enumerate() {
                    let p = &self.map.pics[i];
                    let ratio = if p.width > 0 && p.height > 0 { p.width as f32 / p.height as f32 } else { 1.0 };
                    let (id, v) = (p.id.clone(), p.v.clone());
                    let (r, resp) = ui.allocate_exact_size(vec2((h * ratio).clamp(h * 0.3, h * 3.0), h), Sense::click());
                    if ui.is_rect_visible(r) {
                        self.want_id(ctx, store, &id, v.as_deref(), SMALL);
                        ui.painter().rect_filled(r, 4, LAND);
                        if let Some(t) = self.tex.get(&(id, SMALL), self.frame) {
                            ui.painter().image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                        }
                    }
                    self.map.strip.push(r);
                    if resp.clicked() {
                        open = Some(k);
                    }
                }
            });
        });
        if let Some(k) = open {
            self.open_from_map(k);
        }
    }

    /// Opens picture `k` of the open stack in the viewer (the stack is what it pages through).
    fn open_from_map(&mut self, k: usize) {
        let Some(ix) = self.map.open.clone() else { return };
        self.pics = ix.iter().map(|&i| self.map.pics[i].clone()).collect();
        self.viewer = Some(Viewer::new(k, self.pics.len()));
    }

    /// Test hook: `home:photos:map` {list?, zoom?, lat?, lon?, stack?, picture?, close?}.
    #[cfg(target_arch = "wasm32")]
    pub(in crate::home) fn map_test(&mut self, p: &Json) -> Result<(), String> {
        if let Some(l) = p.get("list").and_then(Json::as_bool) {
            self.map.list = l;
        }
        if let (Some(lat), Some(lon)) = (p.get("lat").and_then(Json::as_f64), p.get("lon").and_then(Json::as_f64)) {
            self.map.centre = mercator(lat, lon);
            self.map.fitted = true;
        }
        if let Some(z) = p.get("zoom").and_then(Json::as_f64) {
            self.map.zoom = (z as f32).clamp(MIN_ZOOM, MAX_ZOOM);
            self.map.fitted = true;
        }
        if let Some(s) = p.get("stack").and_then(Json::as_u64) {
            let ix = self.map.drawn.get(s as usize).ok_or("no such stack on screen")?.1.clone();
            self.map.open = Some(ix);
        }
        if let Some(k) = p.get("picture").and_then(Json::as_u64) {
            let n = self.map.open.as_ref().map_or(0, Vec::len);
            if k as usize >= n {
                return Err("no such picture in the open stack".into());
            }
            self.open_from_map(k as usize);
        }
        if p.get("close").and_then(Json::as_bool) == Some(true) {
            self.map.open = None;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_map_reads_and_fills() {
        let w = world();
        assert!(w.layers.len() >= 6 && w.labels.len() > 100, "{} layers, {} labels", w.layers.len(), w.labels.len());
        for band in 0..2 {
            let f = w.fill(band, 0);
            assert!(f.tris.len() % 3 == 0 && f.tris.len() > 3 * 1000, "band {band}: {} corners", f.tris.len());
            // London is on land, the middle of the Atlantic isn't: count triangles covering each.
            let inside = |t: &[u32], m: [f32; 2]| {
                let [a, b, c] = [f.pts[t[0] as usize], f.pts[t[1] as usize], f.pts[t[2] as usize]];
                let s = [cross(a, b, m), cross(b, c, m), cross(c, a, m)];
                s.iter().all(|v| *v >= 0.0) || s.iter().all(|v| *v <= 0.0)
            };
            let covers = |m: [f32; 2]| f.tris.chunks(3).filter(|t| inside(t, m)).count();
            assert!(covers(mercator(51.5, -0.12)) >= 1, "band {band}: London should be land");
            assert_eq!(covers(mercator(30.0, -40.0)), 0, "band {band}: the Atlantic should be sea");
        }
    }

    #[test]
    fn ear_clipping_covers_a_concave_ring() {
        // An L shape: area 3, four triangles.
        let ring = [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0], [1.0, 2.0], [0.0, 2.0], [0.0, 0.0]];
        let t = triangulate(&ring);
        assert_eq!(t.len(), 12);
        let area: f32 = t.chunks(3).map(|t| cross(ring[t[0] as usize], ring[t[1] as usize], ring[t[2] as usize]).abs() / 2.0).sum();
        assert!((area - 3.0).abs() < 1e-5, "{area}");
    }

    #[test]
    fn stacks_split_as_you_zoom_in() {
        let london = mercator(51.50, -0.12);
        let near = mercator(51.52, -0.10);
        let paris = mercator(48.86, 2.35);
        let pts = [london, near, paris];
        // The world 512 points wide: all three in one stack.
        let mut z = cluster(&pts, 512.0, CLUSTER);
        assert_eq!(z.len(), 1);
        // Country scale: London's two together, Paris apart.
        z = cluster(&pts, 256.0 * 2f32.powi(6), CLUSTER);
        z.sort_by_key(|s| s.1.len());
        assert_eq!(z.iter().map(|s| s.1.len()).collect::<Vec<_>>(), [1, 2]);
        // Street scale: all apart, every picture in exactly one stack.
        z = cluster(&pts, 256.0 * 2f32.powi(16), CLUSTER);
        assert_eq!(z.len(), 3);
        assert_eq!(z.iter().map(|s| s.1.len()).sum::<usize>(), 3);
    }

    #[test]
    fn mercator_corners() {
        assert_eq!(mercator(0.0, 0.0), [0.5, 0.5]);
        let [x, y] = mercator(85.0511, 180.0);
        assert!(x == 1.0 && y < 1e-4, "{x} {y}");
    }
}
