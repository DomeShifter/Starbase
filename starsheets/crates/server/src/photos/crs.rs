//! Our edits' geometry in Adobe's `crs:` and `tiff:` terms (docs/photos-edit.md §2.2–2.3).
//!
//! - **Orientation**: `tiff:Orientation` in our sidecar is the final one: the file's EXIF
//!   orientation, then `rot90` quarter turns clockwise, then `flip_h`. Each orientation is a 2×2
//!   matrix taking centred stored coordinates to centred shown ones (y points down).
//! - **Crop**: `crs:` gives it in the stored picture (before any orientation), as fractions of the
//!   stored width and height: two opposite corners of a rectangle turned by `CropAngle` degrees
//!   about its centre. Ours is a fraction of the largest upright rectangle left after
//!   straightening the shown picture (the renderer's auto-crop, `starbase_photo_edit::output_size`).

use starbase_photo_edit::{Crop, Edits, MAX_ANGLE, MIN_CROP};

/// The picture a sidecar belongs to: its stored pixel size and its file's EXIF orientation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geo {
    pub width: u32,
    pub height: u32,
    pub orientation: u16,
}

type M = [[i32; 2]; 2];

const I: M = [[1, 0], [0, 1]];
/// A quarter turn clockwise (= orientation 6).
const Q: M = [[0, -1], [1, 0]];
/// Mirrored left-right (= orientation 2).
const F: M = [[-1, 0], [0, 1]];

/// EXIF orientation 1..8 as a matrix (anything else = 1).
fn of(o: u16) -> M {
    match o {
        2 => F,
        3 => [[-1, 0], [0, -1]],
        4 => [[1, 0], [0, -1]],
        5 => [[0, 1], [1, 0]],
        6 => Q,
        7 => [[0, -1], [-1, 0]],
        8 => [[0, 1], [-1, 0]],
        _ => I,
    }
}

fn orientation_of(m: M) -> u16 {
    (1..=8).find(|&o| of(o) == m).unwrap_or(1)
}

fn mul(a: M, b: M) -> M {
    let mut out = [[0; 2]; 2];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = a[i][0] * b[0][j] + a[i][1] * b[1][j];
        }
    }
    out
}

fn transposed(a: M) -> M {
    [[a[0][0], a[1][0]], [a[0][1], a[1][1]]]
}

fn det(a: M) -> i32 {
    a[0][0] * a[1][1] - a[0][1] * a[1][0]
}

/// The final orientation: the file's, then the quarter turns, then the flip.
pub fn final_orientation(exif: u16, rot90: u8, flip: bool) -> u16 {
    let mut m = of(exif);
    for _ in 0..rot90 % 4 {
        m = mul(Q, m);
    }
    if flip {
        m = mul(F, m);
    }
    orientation_of(m)
}

/// The quarter turns and flip that take the file's orientation `exif` to `final_o`.
pub fn user_turns(final_o: u16, exif: u16) -> (u8, bool) {
    let u = mul(of(final_o), transposed(of(exif)));
    let (flip, r) = if det(u) < 0 { (true, mul(F, u)) } else { (false, u) };
    let mut m = I;
    for k in 0..4u8 {
        if m == r {
            return (k, flip);
        }
        m = mul(Q, m);
    }
    (0, false)
}

/// The straightened, auto-cropped rectangle (pixels) of a `w` × `h` shown picture.
fn inner(w: u32, h: u32, angle: f32) -> (f64, f64) {
    let (aw, ah) = starbase_photo_edit::output_size(w, h, &Edits { angle, ..Edits::default() });
    (f64::from(aw.max(1)), f64::from(ah.max(1)))
}

/// The shown size (after orientation `m`) of the stored picture.
fn shown(g: &Geo, m: M) -> (u32, u32) {
    if m[0][0] == 0 { (g.height, g.width) } else { (g.width, g.height) }
}

/// `crs:` crop (left, top, right, bottom, angle) for these edits: None when there is no crop.
pub fn crop_to_crs(e: &Edits, g: &Geo) -> Option<[f64; 5]> {
    if e.crop.is_full() && e.angle == 0.0 {
        return None;
    }
    let m = of(final_orientation(g.orientation, e.rot90, e.flip_h));
    let (wf, hf) = shown(g, m);
    let (aw, ah) = inner(wf, hf, e.angle);
    let c = &e.crop;
    let (dx, dy) = ((f64::from(c.l + c.r) / 2.0 - 0.5) * aw, (f64::from(c.t + c.b) / 2.0 - 0.5) * ah);
    let (mut w, mut h) = (f64::from(c.r - c.l) * aw, f64::from(c.b - c.t) * ah);
    // Crop centre in the shown frame: the renderer maps an output point q to C + R·q,
    // R = [[cos, sin], [-sin, cos]].
    let (s, co) = f64::from(e.angle).to_radians().sin_cos();
    let (fx, fy) = (co * dx + s * dy, -s * dx + co * dy);
    // Back to the stored frame (the matrix is orthogonal: its inverse is its transpose).
    let (sx, sy) = (f64::from(m[0][0]) * fx + f64::from(m[1][0]) * fy, f64::from(m[0][1]) * fx + f64::from(m[1][1]) * fy);
    if m[0][0] == 0 {
        std::mem::swap(&mut w, &mut h);
    }
    let a = f64::from(det(m)) * f64::from(e.angle);
    let (sa, ca) = a.to_radians().sin_cos();
    let (ws, hs) = (f64::from(g.width), f64::from(g.height));
    let (cx, cy) = (sx + ws / 2.0, sy + hs / 2.0);
    let rot = |x: f64, y: f64| (ca * x - sa * y, sa * x + ca * y);
    let (tl, br) = (rot(-w / 2.0, -h / 2.0), rot(w / 2.0, h / 2.0));
    Some([(cx + tl.0) / ws, (cy + tl.1) / hs, (cx + br.0) / ws, (cy + br.1) / hs, a])
}

/// Our quarter turns, flip, angle and crop from `crs:` corners `[l, t, r, b, angle]` (None = no
/// crop), the sidecar's final orientation `final_o` and the picture.
pub fn from_crs(crop: Option<[f64; 5]>, final_o: u16, g: &Geo) -> (u8, bool, f32, Crop) {
    let (rot90, flip) = user_turns(final_o, g.orientation);
    let Some([l, t, r, b, a]) = crop else { return (rot90, flip, 0.0, Crop::FULL) };
    let m = of(final_o);
    let (ws, hs) = (f64::from(g.width), f64::from(g.height));
    let (cx, cy) = ((l + r) / 2.0 * ws, (t + b) / 2.0 * hs);
    let (dx, dy) = ((r - l) * ws, (b - t) * hs);
    let (sa, ca) = a.to_radians().sin_cos();
    let (mut w, mut h) = ((dx * ca + dy * sa).abs(), (dy * ca - dx * sa).abs());
    let angle = (f64::from(det(m)) * a).clamp(-f64::from(MAX_ANGLE), f64::from(MAX_ANGLE)) as f32;
    if m[0][0] == 0 {
        std::mem::swap(&mut w, &mut h);
    }
    let (sx, sy) = (cx - ws / 2.0, cy - hs / 2.0);
    let (fx, fy) = (f64::from(m[0][0]) * sx + f64::from(m[0][1]) * sy, f64::from(m[1][0]) * sx + f64::from(m[1][1]) * sy);
    let (s, co) = f64::from(angle).to_radians().sin_cos();
    let (qx, qy) = (co * fx - s * fy, s * fx + co * fy);
    let (wf, hf) = shown(g, m);
    let (aw, ah) = inner(wf, hf, angle);
    let side = |centre: f64, size: f64, span: f64| {
        let lo = (0.5 + centre / span - size / (2.0 * span)).clamp(0.0, 1.0);
        let hi = (0.5 + centre / span + size / (2.0 * span)).clamp(0.0, 1.0);
        (lo as f32, hi as f32)
    };
    let (cl, cr) = side(qx, w, aw);
    let (ct, cb) = side(qy, h, ah);
    let crop = if cr - cl >= MIN_CROP && cb - ct >= MIN_CROP { Crop { l: cl, t: ct, r: cr, b: cb } } else { Crop::FULL };
    (rot90, flip, angle, crop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientations_compose_and_come_apart() {
        assert_eq!(final_orientation(1, 1, false), 6, "a quarter turn clockwise");
        assert_eq!(final_orientation(6, 3, false), 1);
        assert_eq!(final_orientation(1, 0, true), 2);
        for exif in 1..=8 {
            for rot in 0..4 {
                for flip in [false, true] {
                    assert_eq!(user_turns(final_orientation(exif, rot, flip), exif), (rot, flip), "{exif} {rot} {flip}");
                }
            }
        }
    }

    #[test]
    fn crops_go_to_crs_and_back() {
        let g = Geo { width: 600, height: 400, orientation: 6 };
        for (rot90, flip_h, angle) in [(0, false, 0.0), (1, false, 10.0), (2, true, -7.5), (3, true, 30.0)] {
            let e = Edits { rot90, flip_h, angle, crop: Crop { l: 0.1, t: 0.2, r: 0.7, b: 0.9 }, ..Edits::default() };
            let c = crop_to_crs(&e, &g).unwrap();
            let (r, f, a, crop) = from_crs(Some(c), final_orientation(g.orientation, rot90, flip_h), &g);
            assert_eq!((r, f), (rot90, flip_h));
            assert!((a - angle).abs() < 1e-4, "{a} {angle}");
            for (x, y) in [(crop.l, e.crop.l), (crop.t, e.crop.t), (crop.r, e.crop.r), (crop.b, e.crop.b)] {
                assert!((x - y).abs() < 1e-3, "{crop:?} vs {:?} ({rot90} {flip_h} {angle})", e.crop);
            }
        }
    }
}
