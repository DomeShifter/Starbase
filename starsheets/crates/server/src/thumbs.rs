//! Preview pictures for the home screen: the top-left corner of a spreadsheet (its active sheet),
//! drawn on the server so every file has one, also files changed outside StarSheets or never
//! opened here.
//!
//! Cheap on purpose: only about 20 rows × 6 columns are drawn (a few milliseconds), one picture at
//! a time. A file being edited gets a new picture at most once a minute and when the last person
//! leaves, drawn from the copy its live room already holds; other files get theirs the first time
//! someone asks, from the file on disk. Each picture is kept in `.starsheets/thumbs/<id>.png` with
//! the file version it shows (`<id>.ver`), so it's only drawn again after the file changes. Files
//! over MAX_READ get no picture (the app shows a plain icon) rather than being read just for it.
//!
//! Documents (StarDocs) can't be drawn here: StarDocs sends a picture of the top of page 1 after
//! each save (`take`), kept the same way and only shown while it matches the file on disk.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use ab_glyph::{Font as _, FontRef, PxScale, ScaleFont as _};
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_engine::model::{HAlign, PatternType, Workbook};
use tiny_skia::{Color, Paint, Pixmap, Rect, Transform};

use crate::sharing::Role;
use crate::store::{Bad, Store};

/// The picture's size in pixels, and how much smaller than the sheet it's drawn: 720 px at 0.6
/// is 1200 px of sheet, columns A to L at the default 100 px (to-do 2 row 15; was 480 at 0.75).
pub const WIDTH: u32 = 720;
pub const HEIGHT: u32 = 480;
const SCALE: f32 = 0.6;
/// Goes up whenever the drawing changes, so pictures kept by an older server (and browsers'
/// copies, through the ETag) are drawn again rather than shown until the file next changes.
pub const DRAW_TAG: &str = "d2";
/// Files bigger than this aren't read only to draw a picture.
pub const MAX_READ: u64 = 8 * 1024 * 1024;

/// One picture at a time, opening the file included (security review #11): many preview
/// requests at once each unpacking a file (up to GridCraft's 1 GB limit for one package) could
/// otherwise use up the server's memory. Opening only for a picture can't be limited more
/// tightly without a change in GridCraft (its zip limits are fixed constants).
static DRAWING: Mutex<()> = Mutex::new(());
/// Files waiting to be opened for a picture, and the most that may wait.
static WAITING: AtomicUsize = AtomicUsize::new(0);
const MAX_WAITING: usize = 16;

static REGULAR: &[u8] = include_bytes!("../../../app/fonts/Roboto-Regular.ttf");
static BOLD: &[u8] = include_bytes!("../../../app/fonts/Roboto-Bold.ttf");
static ITALIC: &[u8] = include_bytes!("../../../app/fonts/Roboto-Italic.ttf");
static BOLD_ITALIC: &[u8] = include_bytes!("../../../app/fonts/Roboto-BoldItalic.ttf");

fn dir(store: &Store) -> PathBuf {
    store.meta().join("thumbs")
}

/// The kept picture of `file` if it shows `version`.
pub fn cached(store: &Store, file: &str, version: &str) -> Option<Vec<u8>> {
    let d = dir(store);
    (std::fs::read_to_string(d.join(format!("{file}.ver"))).ok()?.trim() == format!("{DRAW_TAG}/{version}"))
        .then(|| std::fs::read(d.join(format!("{file}.png"))).ok())
        .flatten()
}

/// Keeps a picture of `file` at `version`.
pub fn keep(store: &Store, file: &str, version: &str, png: &[u8]) -> std::io::Result<()> {
    let d = dir(store);
    std::fs::create_dir_all(&d)?;
    let tmp = d.join(format!(".{file}.tmp"));
    std::fs::write(&tmp, png)?;
    std::fs::rename(&tmp, d.join(format!("{file}.png")))?;
    std::fs::write(d.join(format!("{file}.ver")), format!("{DRAW_TAG}/{version}"))
}

/// Biggest picture StarDocs may send, in bytes and pixels (twice the size we show).
pub const MAX_UPLOAD: usize = 1024 * 1024;
pub const MAX_UPLOAD_W: u32 = 2 * WIDTH;
pub const MAX_UPLOAD_H: u32 = 2 * HEIGHT;

/// Width and height of a PNG, from its header (no decoding), or why it isn't one.
pub fn png_size(png: &[u8]) -> Result<(u32, u32), &'static str> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if png.len() < 33 || !png.starts_with(SIGNATURE) || &png[12..16] != b"IHDR" {
        return Err("That isn't a PNG picture.");
    }
    let be = |i: usize| u32::from_be_bytes([png[i], png[i + 1], png[i + 2], png[i + 3]]);
    Ok((be(16), be(20)))
}

/// A picture of document `file` (at `path`) sent by StarDocs after saving `version`; `role` is
/// what the sender may do with the file. Kept only if it still shows the file on disk.
pub fn take(store: &Store, role: Role, file: &str, path: &str, version: &str, png: &[u8]) -> Result<(), Bad> {
    if role == Role::None {
        return Err(Bad::new(403, "You don't have access to that."));
    }
    if !crate::store::is_doc(path) {
        return Err(Bad::new(400, "Only documents take a picture; spreadsheets are drawn by the server."));
    }
    if role < Role::Edit {
        return Err(Bad::new(403, "You can only view that."));
    }
    if png.len() > MAX_UPLOAD {
        return Err(Bad::new(413, "picture too large"));
    }
    let (w, h) = png_size(png).map_err(|e| Bad::new(400, e))?;
    if w == 0 || h == 0 || w > MAX_UPLOAD_W || h > MAX_UPLOAD_H {
        return Err(Bad::new(400, format!("The picture must be at most {MAX_UPLOAD_W} × {MAX_UPLOAD_H} pixels.")));
    }
    let now = store.version_of(&store.full(path)).map_err(|_| Bad::new(404, "no such file"))?;
    if now != version {
        return Err(Bad::new(409, "The document changed since; this picture is out of date."));
    }
    keep(store, file, version, png).map_err(|_| Bad::new(500, "couldn't keep the picture"))
}

pub fn forget(store: &Store, file: &str) {
    let d = dir(store);
    let _ = std::fs::remove_file(d.join(format!("{file}.png")));
    let _ = std::fs::remove_file(d.join(format!("{file}.ver")));
}

/// Opens a spreadsheet file's bytes (xlsx, csv, ods…) and draws it.
pub fn from_bytes(name: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    // Each waiting request holds its file's bytes (up to MAX_READ): past a few, say "not now"
    // (the app shows a plain icon and asks again another time).
    struct Waiting;
    impl Drop for Waiting {
        fn drop(&mut self) {
            WAITING.fetch_sub(1, Ordering::Relaxed);
        }
    }
    if WAITING.fetch_add(1, Ordering::Relaxed) >= MAX_WAITING {
        WAITING.fetch_sub(1, Ordering::Relaxed);
        return Err("too many pictures waiting".into());
    }
    let waiting = Waiting;
    let _one = DRAWING.lock().unwrap_or_else(|e| e.into_inner());
    drop(waiting);
    let mut s = gridcraft_engine::Session::new();
    let b64 = gridcraft_engine::io::base64_encode(bytes);
    s.execute("file.open", serde_json::json!({ "name": name, "base64": b64 })).map_err(|e| e.to_string())?;
    let wb = s.active().map(|d| d.wb.clone()).ok_or("nothing opened")?;
    drop(s);
    draw_one(&wb)
}

fn rgb(c: [u8; 3]) -> Color {
    Color::from_rgba8(c[0], c[1], c[2], 255)
}

/// Draws the top-left corner of the active sheet as a PNG.
pub fn draw(wb: &Workbook) -> Result<Vec<u8>, String> {
    let _one = DRAWING.lock().unwrap_or_else(|e| e.into_inner());
    draw_one(wb)
}

/// [`draw`], with DRAWING already held.
fn draw_one(wb: &Workbook) -> Result<Vec<u8>, String> {
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

#[cfg(test)]
mod tests {
    #[test]
    fn draws_a_small_png() {
        let png = super::from_bytes("t.csv", b"Name,Amount\nRent,1200\nFood,350.5\nA long piece of text that runs on,\n").unwrap();
        assert_eq!(&png[1..4], b"PNG");
        assert!(png.len() < 60_000, "{} bytes", png.len());
        // Something dark was drawn (the text), not just a white page with lines.
        let pm = tiny_skia::Pixmap::decode_png(&png).unwrap();
        assert!(pm.data().chunks(4).any(|p| p[0] < 100), "no text");
        assert_eq!((pm.width(), pm.height()), (super::WIDTH, super::HEIGHT));
    }

    use crate::sharing::Role;
    use crate::store::Store;

    fn png(w: u32, h: u32) -> Vec<u8> {
        tiny_skia::Pixmap::new(w, h).unwrap().encode_png().unwrap()
    }

    #[test]
    fn takes_a_documents_picture() {
        let dir = std::env::temp_dir().join(format!("starsheets-thumbs-{}", crate::store::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Letter.docx"), b"doc").unwrap();
        std::fs::write(dir.join("a.xlsx"), b"sheet").unwrap();
        let store = Store { root: dir.clone(), keep: 3, spaces: false };
        let v = store.version_of(&store.full("Letter.docx")).unwrap();
        let pic = png(super::WIDTH, super::HEIGHT);
        let code = |role, path: &str, version: &str, png: &[u8]| super::take(&store, role, "d1", path, version, png).err().map(|b| b.code);
        assert_eq!(code(Role::Edit, "a.xlsx", &v, &pic), Some(400), "not a document");
        assert_eq!(code(Role::View, "Letter.docx", &v, &pic), Some(403), "viewer");
        assert_eq!(code(Role::None, "Letter.docx", &v, &pic), Some(403), "no access");
        assert_eq!(code(Role::Edit, "Letter.docx", "1-2", &pic), Some(409), "older version");
        assert_eq!(code(Role::Edit, "Letter.docx", &v, b"GIF89a, not a png at all, honestly"), Some(400), "not a png");
        assert_eq!(code(Role::Edit, "Letter.docx", &v, &png(super::MAX_UPLOAD_W + 1, 10)), Some(400), "too wide");
        assert_eq!(code(Role::Edit, "Letter.docx", &v, &png(10, super::MAX_UPLOAD_H + 1)), Some(400), "too tall");
        let mut zero = pic.clone();
        zero[16..20].copy_from_slice(&0u32.to_be_bytes());
        assert_eq!(code(Role::Edit, "Letter.docx", &v, &zero), Some(400), "zero width");
        let mut big = pic.clone();
        big.resize(super::MAX_UPLOAD + 1, 0);
        assert_eq!(code(Role::Edit, "Letter.docx", &v, &big), Some(413), "too big");
        assert_eq!(super::cached(&store, "d1", &v), None, "nothing kept from refused pictures");
        assert_eq!(code(Role::Edit, "Letter.docx", &v, &pic), None);
        assert_eq!(super::cached(&store, "d1", &v), Some(pic));
        assert_eq!(super::png_size(&png(3, 7)), Ok((3, 7)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// THUMB_DIR=folder cargo test … draws every spreadsheet in that folder to <name>.png (by eye).
    #[test]
    fn draw_folder_for_a_look() {
        let Ok(dir) = std::env::var("THUMB_DIR") else { return };
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.ends_with(".xlsx") {
                let t = std::time::Instant::now();
                let png = super::from_bytes(&name, &std::fs::read(e.path()).unwrap()).unwrap();
                eprintln!("{name}: {} bytes in {:?}", png.len(), t.elapsed());
                std::fs::write(format!("{dir}/{name}.png"), png).unwrap();
            }
        }
    }
}
