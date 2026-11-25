//! Quarter turns, flip, straighten and crop. Without a straighten angle every step copies whole
//! pixels (exact); with one, straighten and crop are a single bilinear resampling pass.

use std::borrow::Cow;

use image::{RgbaImage, imageops};

use crate::model::{Crop, Edits};

/// Size after the quarter turns.
fn turned(w: u32, h: u32, rot90: u8) -> (u32, u32) {
    if rot90 % 2 == 1 { (h, w) } else { (w, h) }
}

/// The largest axis-aligned rectangle inside a `w` × `h` rectangle turned by `angle` degrees
/// (whole pixels, at least 1 × 1, never bigger than the original).
pub(crate) fn straightened(w: u32, h: u32, angle: f32) -> (u32, u32) {
    if angle == 0.0 {
        return (w, h);
    }
    let (wf, hf) = (w as f64, h as f64);
    let a = (angle as f64).to_radians();
    let (sin_a, cos_a) = (a.sin().abs(), a.cos().abs());
    let (long, short) = if w >= h { (wf, hf) } else { (hf, wf) };
    let (rw, rh) = if short <= 2.0 * sin_a * cos_a * long || (sin_a - cos_a).abs() < 1e-10 {
        // Half-constrained: two corners touch the longer sides.
        let x = 0.5 * short;
        if w >= h { (x / sin_a, x / cos_a) } else { (x / cos_a, x / sin_a) }
    } else {
        let cos_2a = cos_a * cos_a - sin_a * sin_a;
        ((wf * cos_a - hf * sin_a) / cos_2a, (hf * cos_a - wf * sin_a) / cos_2a)
    };
    // The tiny epsilon keeps an exact fit (e.g. 100.0000000001) from rounding down a pixel.
    let px = |v: f64, max: u32| ((v + 1e-9).floor().max(1.0) as u32).min(max);
    (px(rw, w), px(rh, h))
}

/// Crop rectangle in whole pixels (x, y, width, height) of a `w` × `h` image; never empty.
pub(crate) fn crop_rect(w: u32, h: u32, c: &Crop) -> (u32, u32, u32, u32) {
    let span = |lo: f32, hi: f32, n: u32| {
        let a = ((lo as f64 * n as f64).round() as u32).min(n - 1);
        let b = ((hi as f64 * n as f64).round() as u32).clamp(a + 1, n);
        (a, b - a)
    };
    let (x, cw) = span(c.l, c.r, w);
    let (y, ch) = span(c.t, c.b, h);
    (x, y, cw, ch)
}

pub(crate) fn output_size(w: u32, h: u32, e: &Edits) -> (u32, u32) {
    let (w, h) = turned(w, h, e.rot90);
    let (sw, sh) = straightened(w, h, e.angle);
    let (_, _, cw, ch) = crop_rect(sw, sh, &e.crop);
    (cw, ch)
}

pub(crate) fn apply(img: &RgbaImage, e: &Edits) -> RgbaImage {
    let mut cur: Cow<RgbaImage> = Cow::Borrowed(img);
    cur = match e.rot90 % 4 {
        1 => Cow::Owned(imageops::rotate90(&*cur)),
        2 => Cow::Owned(imageops::rotate180(&*cur)),
        3 => Cow::Owned(imageops::rotate270(&*cur)),
        _ => cur,
    };
    if e.flip_h {
        cur = Cow::Owned(imageops::flip_horizontal(&*cur));
    }
    if e.angle == 0.0 {
        if e.crop.is_full() {
            return cur.into_owned();
        }
        let (x, y, cw, ch) = crop_rect(cur.width(), cur.height(), &e.crop);
        return imageops::crop_imm(&*cur, x, y, cw, ch).to_image();
    }
    straighten_crop(&cur, e.angle, &e.crop)
}

/// Turns the picture `angle` degrees clockwise about its centre and takes the crop of the
/// largest inner rectangle, sampling bilinearly.
fn straighten_crop(src: &RgbaImage, angle: f32, crop: &Crop) -> RgbaImage {
    let (w, h) = src.dimensions();
    let (sw, sh) = straightened(w, h, angle);
    let (x0, y0, cw, ch) = crop_rect(sw, sh, crop);
    let a = (angle as f64).to_radians();
    let (s, c) = (a.sin() as f32, a.cos() as f32);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let (hx, hy) = (sw as f32 / 2.0, sh as f32 / 2.0);
    let (maxx, maxy) = ((w - 1) as f32, (h - 1) as f32);
    let data = src.as_raw();
    let stride = w as usize * 4;
    let mut out = vec![0u8; cw as usize * ch as usize * 4];
    for (j, row) in out.chunks_exact_mut(cw as usize * 4).enumerate() {
        let py = (y0 as usize + j) as f32 + 0.5 - hy;
        for (i, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let qx = (x0 as usize + i) as f32 + 0.5 - hx;
            // Output point → source point: rotate back by the angle (y points down).
            let sx = (cx + c * qx + s * py - 0.5).clamp(0.0, maxx);
            let sy = (cy - s * qx + c * py - 0.5).clamp(0.0, maxy);
            let (xf, yf) = (sx.floor(), sy.floor());
            let (fx, fy) = (sx - xf, sy - yf);
            let (xi, yi) = (xf as usize, yf as usize);
            let xn = if xi + 1 < w as usize { 4 } else { 0 };
            let yn = if yi + 1 < h as usize { stride } else { 0 };
            let p00 = yi * stride + xi * 4;
            let (w00, w10, w01, w11) = ((1.0 - fx) * (1.0 - fy), fx * (1.0 - fy), (1.0 - fx) * fy, fx * fy);
            for (k, o) in px.iter_mut().enumerate() {
                let v = data[p00 + k] as f32 * w00
                    + data[p00 + xn + k] as f32 * w10
                    + data[p00 + yn + k] as f32 * w01
                    + data[p00 + yn + xn + k] as f32 * w11;
                *o = (v + 0.5) as u8;
            }
        }
    }
    RgbaImage::from_raw(cw, ch, out).expect("buffer is cw*ch*4")
}
