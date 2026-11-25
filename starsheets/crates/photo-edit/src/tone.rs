//! Tone, colour and filters: one pass over the pixels. Steps left at zero are skipped entirely
//! (an untouched channel stays exactly `c / 255`, which rounds back to `c`).

use image::RgbaImage;

use crate::model::{Edits, FilterName};

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// S-curve around mid-grey; `k` in -1..=1 (negative flattens). Keeps 0, 0.5 and 1 in place and
/// stays monotonic.
fn contrast_curve(y: f32, k: f32) -> f32 {
    if k == 0.0 {
        return y;
    }
    let target = if k > 0.0 {
        if y < 0.5 { 2.0 * y * y } else { 1.0 - 2.0 * (1.0 - y) * (1.0 - y) }
    } else if y < 0.5 {
        (y / 2.0).sqrt()
    } else {
        1.0 - ((1.0 - y) / 2.0).sqrt()
    };
    y + (target - y) * k.abs()
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Rec. 709 weights (on sRGB values: a cheap "perceptual" lightness).
fn luma(r: f32, g: f32, b: f32) -> f32 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// Change of lightness for highlights (`h`) and shadows (`s`), both -1..=1. Highlights act above
/// mid-grey, shadows below, faded in smoothly. The 0.5 keeps them monotonic at full strength
/// (the steepest mask term is 1.6875, so any factor under 0.59 works).
fn highlights_shadows(l: f32, h: f32, s: f32) -> f32 {
    let mut d = 0.0;
    if h != 0.0 {
        let m = smoothstep(0.5, 1.0, l);
        d += 0.5 * h * m * if h > 0.0 { 1.0 - l } else { l - 0.5 };
    }
    if s != 0.0 {
        let m = 1.0 - smoothstep(0.0, 0.5, l);
        d += 0.5 * s * m * if s > 0.0 { 0.5 - l } else { l };
    }
    d
}

/// Pushes colour away from (k > 1) or towards (k < 1) grey of the same lightness.
fn saturate(r: f32, g: f32, b: f32, k: f32) -> (f32, f32, f32) {
    let y = luma(r, g, b);
    (y + (r - y) * k, y + (g - y) * k, y + (b - y) * k)
}

fn filter(name: FilterName, r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    match name {
        FilterName::Bw => {
            let y = 0.299 * r + 0.587 * g + 0.114 * b;
            (y, y, y)
        }
        FilterName::Sepia => (0.393 * r + 0.769 * g + 0.189 * b, 0.349 * r + 0.686 * g + 0.168 * b, 0.272 * r + 0.534 * g + 0.131 * b),
        FilterName::Warm => (r * 1.06 + 0.03, g * 1.01, b * 0.88),
        FilterName::Cool => (r * 0.9, g, b * 1.06 + 0.03),
        FilterName::Vivid => {
            let (r, g, b) = saturate(r, g, b, 1.4);
            (contrast_curve(r.clamp(0.0, 1.0), 0.2), contrast_curve(g.clamp(0.0, 1.0), 0.2), contrast_curve(b.clamp(0.0, 1.0), 0.2))
        }
        FilterName::Fade => {
            let (r, g, b) = saturate(r, g, b, 0.75);
            (0.08 + 0.86 * r, 0.08 + 0.86 * g, 0.08 + 0.86 * b)
        }
    }
}

fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

pub(crate) fn apply(img: &mut RgbaImage, e: &Edits) {
    // Exposure (linear light) and contrast are per channel: one 256-entry table.
    let mut lut = [0f32; 256];
    let ev = 2f32.powf(e.exposure);
    let k = e.contrast as f32 / 100.0;
    for (c, v) in lut.iter_mut().enumerate() {
        let mut y = c as f32 / 255.0;
        if e.exposure != 0.0 {
            y = linear_to_srgb((srgb_to_linear(y) * ev).min(1.0));
        }
        *v = contrast_curve(y, k);
    }
    let (hl, sh) = (e.highlights as f32 / 100.0, e.shadows as f32 / 100.0);
    let hs = hl != 0.0 || sh != 0.0;
    let (t, ti) = (e.temperature as f32 / 100.0, e.tint as f32 / 100.0);
    let gains = (t != 0.0 || ti != 0.0).then_some((1.0 + 0.15 * t + 0.05 * ti, 1.0 - 0.12 * ti, 1.0 - 0.15 * t + 0.05 * ti));
    let (sat, vib) = (e.saturation as f32 / 100.0, e.vibrance as f32 / 100.0);
    let colour = sat != 0.0 || vib != 0.0;
    let filt = e.filter.filter(|f| f.amount > 0.0);

    for p in img.as_mut().as_chunks_mut::<4>().0 {
        let (mut r, mut g, mut b) = (lut[p[0] as usize], lut[p[1] as usize], lut[p[2] as usize]);
        if hs {
            let d = highlights_shadows(luma(r, g, b).clamp(0.0, 1.0), hl, sh);
            (r, g, b) = (r + d, g + d, b + d);
        }
        if let Some((gr, gg, gb)) = gains {
            (r, g, b) = (r * gr, g * gg, b * gb);
        }
        if colour {
            (r, g, b) = (r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0));
            // Vibrance lifts muted colours more than strong ones.
            let chroma = r.max(g).max(b) - r.min(g).min(b);
            (r, g, b) = saturate(r, g, b, (1.0 + sat) * (1.0 + vib * (1.0 - chroma)));
        }
        if let Some(f) = filt {
            let (r0, g0, b0) = (r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0));
            let (fr, fg, fb) = filter(f.name, r0, g0, b0);
            let a = f.amount;
            (r, g, b) = (r0 + (fr - r0) * a, g0 + (fg - g0) * a, b0 + (fb - b0) * a);
        }
        p[0] = to_u8(r);
        p[1] = to_u8(g);
        p[2] = to_u8(b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_are_monotonic_and_keep_ends() {
        for k in [-1.0, -0.5, 0.3, 1.0] {
            assert_eq!(contrast_curve(0.0, k), 0.0);
            assert!((contrast_curve(0.5, k) - 0.5).abs() < 1e-6);
            assert!((contrast_curve(1.0, k) - 1.0).abs() < 1e-6);
            let mut prev = -1.0;
            for i in 0..=1000 {
                let v = contrast_curve(i as f32 / 1000.0, k);
                assert!(v >= prev - 1e-6, "contrast {k} not monotonic at {i}");
                prev = v;
            }
        }
        for (h, s) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0), (1.0, 1.0), (-1.0, -1.0)] {
            let mut prev = -1.0;
            for i in 0..=1000 {
                let l = i as f32 / 1000.0;
                let v = l + highlights_shadows(l, h, s);
                assert!(v >= prev - 1e-6, "h/s {h},{s} not monotonic at {l}");
                assert!((-1e-6..=1.0 + 1e-6).contains(&v));
                prev = v;
            }
        }
    }

    #[test]
    fn srgb_round_trip() {
        for c in 0..=255u8 {
            let v = c as f32 / 255.0;
            assert_eq!(to_u8(linear_to_srgb(srgb_to_linear(v))), c);
        }
    }
}
