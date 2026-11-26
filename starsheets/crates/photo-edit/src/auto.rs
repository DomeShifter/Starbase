//! One-click fixes that compute ordinary edit values: `auto_enhance` (levels stretch, a little
//! contrast, grey-world white balance) and `apply_red_eye` (used by `render`).

use image::RgbaImage;

use crate::model::{Edits, RedEye};

/// Share of pixels clipped at each end when the levels are stretched.
const CLIP: f32 = 0.005;

/// Edits for `img` (the untouched picture) with the exposure, contrast, shadows, temperature and
/// tint sliders set from its histogram; everything else in `base` is kept. Deterministic.
pub fn auto_enhance(img: &RgbaImage, base: &Edits) -> Edits {
    let mut e = base.clone();
    (e.exposure, e.contrast, e.shadows, e.temperature, e.tint) = (0.0, 0, 0, 0, 0);
    // Sample at most ~250k pixels on a fixed stride so big pictures cost little.
    let n = img.width() as usize * img.height() as usize;
    if n == 0 {
        return e;
    }
    let step = (n / 250_000).max(1);
    let (mut hist, mut sum, mut cnt) = ([0u32; 256], [0f64; 3], 0u32);
    for p in img.pixels().step_by(step) {
        if p[3] == 0 {
            continue;
        }
        let l = 0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32;
        hist[l.round().clamp(0.0, 255.0) as usize] += 1;
        for c in 0..3 {
            sum[c] += p[c] as f64;
        }
        cnt += 1;
    }
    if cnt == 0 {
        return e;
    }
    let edge = ((cnt as f32 * CLIP) as u32).max(1);
    let pct = |from_top: bool| -> f32 {
        let mut acc = 0;
        for i in 0..256 {
            let v = if from_top { 255 - i } else { i };
            acc += hist[v];
            if acc > edge {
                return v as f32 / 255.0;
            }
        }
        0.5
    };
    let (lo, hi) = (pct(false), pct(true));
    if hi - lo < 0.02 {
        return e; // a flat picture: nothing to stretch
    }
    // White point to 0.97 by brightness (EV, in linear light), black point down by Shadows.
    let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    e.exposure = (lin(0.97) / lin(hi.max(0.05))).log2().clamp(-2.0, 2.0);
    e.exposure = (e.exposure * 100.0).round() / 100.0;
    e.shadows = -((lo * 220.0).round().clamp(0.0, 60.0) as i32);
    // Mild contrast, more for a narrow range.
    e.contrast = ((0.85 - (hi - lo)) * 60.0).round().clamp(0.0, 30.0) as i32;
    // Grey-world: pull the red/blue and green/magenta balance towards neutral, at 70 %.
    let (mr, mg, mb) = ((sum[0] / cnt as f64) as f32, (sum[1] / cnt as f64) as f32, (sum[2] / cnt as f64) as f32);
    if mr + mb > 1.0 && mg > 1.0 {
        let t = (mb - mr) / (0.15 * (mr + mb));
        let rb = (mr + mb) / 2.0;
        let ti = (mg - rb) / (0.12 * mg + 0.05 * rb);
        e.temperature = (t * 70.0).round().clamp(-40.0, 40.0) as i32;
        e.tint = (ti * 70.0).round().clamp(-40.0, 40.0) as i32;
    }
    e
}

/// Desaturates red pixels inside each spot: where r > 1.5 * max(g, b) the pixel becomes the dark
/// grey of its green and blue.
pub(crate) fn apply_red_eye(img: &mut RgbaImage, spots: &[RedEye]) {
    let (w, h) = (img.width() as f32, img.height() as f32);
    let side = w.min(h);
    for s in spots {
        let (cx, cy, r) = (s.x * w, s.y * h, (s.r * side).max(1.0));
        let x0 = (cx - r).floor().max(0.0) as u32;
        let x1 = ((cx + r).ceil().max(0.0) as u32).min(img.width().saturating_sub(1));
        let y0 = (cy - r).floor().max(0.0) as u32;
        let y1 = ((cy + r).ceil().max(0.0) as u32).min(img.height().saturating_sub(1));
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                if dx * dx + dy * dy > r * r {
                    continue;
                }
                let p = img.get_pixel_mut(x, y);
                let (pr, pg, pb) = (p[0] as f32, p[1] as f32, p[2] as f32);
                if pr > 1.5 * pg.max(pb) {
                    let v = ((pg + pb) / 2.0).round() as u8;
                    p[0] = v;
                    p[1] = v;
                    p[2] = v;
                }
            }
        }
    }
}
