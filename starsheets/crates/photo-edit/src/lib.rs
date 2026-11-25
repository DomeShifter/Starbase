//! Starbase Photos' non-destructive editor: the edit model and the renderer.
//!
//! Pure Rust, no I/O, no threads: the web app previews edits live on the 1024 px thumbnail
//! (wasm32), the server renders thumbnails and exports (native). The original file is never
//! changed; edits are stored next to it (an XMP sidecar, written by the server).
//!
//! Order of operations (fixed): rot90 → flip → straighten → crop → tone (exposure, contrast,
//! highlights/shadows) → colour (temperature/tint, saturation/vibrance) → filter. The EXIF
//! orientation comes before all of these: `render` expects an image already turned upright.
//!
//! The results approximate Lightroom's sliders of the same names; they are not the same maths
//! (darktable's Lightroom import makes the same compromise). Exposure works in linear light;
//! the other tone and colour steps work on sRGB values.

#![forbid(unsafe_code)]

mod geometry;
mod model;
mod tone;

pub use image::RgbaImage;
pub use model::{Aspect, Crop, EditError, Edits, Filter, FilterName, MAX_ANGLE, MAX_EXPOSURE, MAX_SLIDER, MIN_CROP, VERSION};

/// Renders `e` on `img` (already upright). Edits made in code are clamped to their ranges first,
/// so this never panics; untouched steps are skipped, so identity edits give the same bytes back.
pub fn render(img: &RgbaImage, e: &Edits) -> RgbaImage {
    let e = sanitize(e);
    if img.width() == 0 || img.height() == 0 {
        return img.clone();
    }
    let mut out = geometry::apply(img, &e);
    if !e.pixels_are_identity() {
        tone::apply(&mut out, &e);
    }
    out
}

/// The size `render` gives for a `w` × `h` image (both at least 1 when `w` and `h` are).
pub fn output_size(w: u32, h: u32, e: &Edits) -> (u32, u32) {
    if w == 0 || h == 0 {
        return (w, h);
    }
    let e = sanitize(e);
    geometry::output_size(w, h, &e)
}

/// Brings hand-made edits into range: non-finite numbers become "no change", a bad crop becomes
/// the whole picture.
fn sanitize(e: &Edits) -> Edits {
    let fin = |x: f32, lo: f32, hi: f32, d: f32| if x.is_finite() { x.clamp(lo, hi) } else { d };
    let s = |x: i32| x.clamp(-MAX_SLIDER, MAX_SLIDER);
    let mut crop =
        Crop { l: fin(e.crop.l, 0.0, 1.0, 0.0), t: fin(e.crop.t, 0.0, 1.0, 0.0), r: fin(e.crop.r, 0.0, 1.0, 1.0), b: fin(e.crop.b, 0.0, 1.0, 1.0) };
    if crop.r - crop.l < MIN_CROP * 0.5 || crop.b - crop.t < MIN_CROP * 0.5 {
        crop = Crop::FULL;
    }
    Edits {
        rot90: e.rot90 % 4,
        flip_h: e.flip_h,
        angle: fin(e.angle, -MAX_ANGLE, MAX_ANGLE, 0.0),
        crop,
        aspect: e.aspect,
        exposure: fin(e.exposure, -MAX_EXPOSURE, MAX_EXPOSURE, 0.0),
        contrast: s(e.contrast),
        highlights: s(e.highlights),
        shadows: s(e.shadows),
        saturation: s(e.saturation),
        vibrance: s(e.vibrance),
        temperature: s(e.temperature),
        tint: s(e.tint),
        filter: e.filter.map(|f| Filter { name: f.name, amount: fin(f.amount, 0.0, 1.0, 0.0) }),
    }
}
