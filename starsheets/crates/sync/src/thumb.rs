//! Preview pictures of spreadsheets: the top-left corner of the active sheet as a PNG. Shared by
//! the server (crates/server/src/thumbs.rs keeps and serves them) and the desktop app's local
//! Drive (app/src/localstore.rs). Only built with the `thumbs` feature.

use ab_glyph::{Font as _, FontRef, PxScale, ScaleFont as _};
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_engine::model::{HAlign, PatternType, Workbook};
use tiny_skia::{Color, Paint, Pixmap, Rect, Transform};

/// The picture's size in pixels, and how much smaller than the sheet it's drawn: 720 px at 0.6
/// is 1200 px of sheet, columns A to L at the default 100 px (to-do 2 row 15; was 480 at 0.75).
pub const WIDTH: u32 = 720;
pub const HEIGHT: u32 = 480;
const SCALE: f32 = 0.6;

static REGULAR: &[u8] = include_bytes!("../../../app/fonts/Roboto-Regular.ttf");
static BOLD: &[u8] = include_bytes!("../../../app/fonts/Roboto-Bold.ttf");
static ITALIC: &[u8] = include_bytes!("../../../app/fonts/Roboto-Italic.ttf");
static BOLD_ITALIC: &[u8] = include_bytes!("../../../app/fonts/Roboto-BoldItalic.ttf");

fn rgb(c: [u8; 3]) -> Color {
    Color::from_rgba8(c[0], c[1], c[2], 255)
}

/// Draws the top-left corner of the active sheet as a PNG.
pub fn draw(wb: &Workbook) -> Result<Vec<u8>, String> {
    let mut pm = Pixmap::new(WIDTH, HEIGHT).ok_or("no picture")?;
    pm.fill(Color::WHITE);
    let Some(sh) = wb.sheet(wb.active_sheet()).or_else(|| wb.sheet(0)) else { return encode(&pm) };
    let fonts = [REGULAR, BOLD, ITALIC, BOLD_ITALIC].map(|b| FontRef::try_from_slice(b).map_err(|e| e.to_string()));
    let fonts: Vec<FontRef> = fonts.into_iter().collect::<Result<_, _>>()?;
    // Column and row edges that fit.
    let (w, h) = (WIDTH as f32, HEIGHT as f32);
    let mut cols = vec![(0u32, 0.0f32)];
    let mut x = 0.0;
    for c in 0..200u32 {
        x += sh.col_width(c) * SCALE;
        cols.push((c + 1, x));
        if x >= w {
            break;
        }
    }
    let mut rows = vec![(0u32, 0.0f32)];
    let mut y = 0.0;
    for r in 0..400u32 {
        y += sh.row_height(r) * SCALE;
        rows.push((r + 1, y));
        if y >= h {
            break;
        }
    }
    let mut paint = Paint::default();
    // Fills first, then grid lines, then text on top.
    for ri in 0..rows.len() - 1 {
        for ci in 0..cols.len() - 1 {
            let c = CellRef { row: rows[ri].0, col: cols[ci].0 };
            let style = wb.styles.get(sh.style_id(c));
            let fill = match style.fill.pattern {
                PatternType::None => None,
                _ => style.fill.fg.resolve(&wb.theme),
            };
            if let Some(f) = fill
                && let Some(r) = Rect::from_ltrb(cols[ci].1, rows[ri].1, cols[ci + 1].1, rows[ri + 1].1)
            {
                paint.set_color(rgb(f));
                pm.fill_rect(r, &paint, Transform::identity(), None);
            }
        }
    }
    paint.set_color(Color::from_rgba8(0xE1, 0xE3, 0xE1, 255));
    for &(_, x) in &cols[1..] {
        if let Some(r) = Rect::from_xywh(x.floor(), 0.0, 1.0, h) {
            pm.fill_rect(r, &paint, Transform::identity(), None);
        }
    }
    for &(_, y) in &rows[1..] {
        if let Some(r) = Rect::from_xywh(0.0, y.floor(), w, 1.0) {
            pm.fill_rect(r, &paint, Transform::identity(), None);
        }
    }
    for ri in 0..rows.len() - 1 {
        for ci in 0..cols.len() - 1 {
            let c = CellRef { row: rows[ri].0, col: cols[ci].0 };
            let v = sh.value(c);
            if matches!(v, Value::Empty) {
                continue;
            }
            let text = gridcraft_engine::display::cell_text(wb, sh, c);
            if text.is_empty() {
                continue;
            }
            let style = wb.styles.get(sh.style_id(c));
            let font = &fonts[usize::from(style.font.bold) + 2 * usize::from(style.font.italic)];
            let size = (style.font.size.max(6.0) * 96.0 / 72.0 * SCALE).min(40.0);
            let colour = style.font.color.resolve(&wb.theme).unwrap_or([0x20, 0x21, 0x24]);
            let right = match style.align.h {
                HAlign::Right => true,
                HAlign::General => matches!(v, Value::Number(_)),
                _ => false,
            };
            let center = matches!(style.align.h, HAlign::Center | HAlign::CenterAcross);
            let (x0, x1) = (cols[ci].1, cols[ci + 1].1);
            // Left-aligned text runs on over empty cells to its right, as in a sheet.
            let mut clip_right = x1;
            if !right && !center {
                let mut k = ci + 1;
                while k < cols.len() - 1 && matches!(sh.value(CellRef { row: c.row, col: cols[k].0 }), Value::Empty) {
                    k += 1;
                }
                clip_right = cols[k].1;
            }
            let (y0, y1) = (rows[ri].1, rows[ri + 1].1);
            text_at(&mut pm, font, size, &text, (x0, x1, clip_right), (y0, y1), right, center, colour);
        }
    }
    encode(&pm)
}

#[allow(clippy::too_many_arguments)]
fn text_at(
    pm: &mut Pixmap,
    font: &FontRef,
    size: f32,
    text: &str,
    (x0, x1, clip_right): (f32, f32, f32),
    (y0, y1): (f32, f32),
    right: bool,
    center: bool,
    colour: [u8; 3],
) {
    let sf = font.as_scaled(PxScale::from(size));
    let width: f32 = text.chars().map(|ch| sf.h_advance(sf.glyph_id(ch))).sum();
    let pad = 3.0 * SCALE;
    let mut pen = if right {
        x1 - pad - width
    } else if center {
        (x0 + x1 - width) / 2.0
    } else {
        x0 + pad
    };
    let (clip_l, clip_r) = (x0, clip_right);
    // Bottom-aligned, like a sheet: the baseline sits just above the cell's lower edge.
    let baseline = y1 - pad - sf.descent().abs();
    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let data = pm.data_mut();
    for ch in text.chars() {
        let id = sf.glyph_id(ch);
        let glyph = id.with_scale_and_position(PxScale::from(size), ab_glyph::point(pen, baseline));
        pen += sf.h_advance(id);
        let Some(outline) = font.outline_glyph(glyph) else { continue };
        let b = outline.px_bounds();
        outline.draw(|gx, gy, cov| {
            let (px, py) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
            if px < 0 || py < 0 || px >= pw || py >= ph || (px as f32) < clip_l || (px as f32) >= clip_r || (py as f32) < y0 || (py as f32) >= y1 {
                return;
            }
            let i = ((py * pw + px) * 4) as usize;
            let a = cov.clamp(0.0, 1.0);
            for k in 0..3 {
                data[i + k] = (data[i + k] as f32 * (1.0 - a) + colour[k] as f32 * a) as u8;
            }
        });
    }
}

/// PNG without the alpha channel (always opaque) and with strong compression: about a third
/// smaller than tiny-skia's own.
fn encode(pm: &Pixmap) -> Result<Vec<u8>, String> {
    let rgb: Vec<u8> = pm.data().as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, pm.width(), pm.height());
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::High);
    enc.set_filter(png::Filter::Adaptive);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(&rgb).map_err(|e| e.to_string())?;
    w.finish().map_err(|e| e.to_string())?;
    Ok(out)
}
