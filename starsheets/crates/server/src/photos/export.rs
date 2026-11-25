//! "Save a copy": a picture exported with its edits as a new file (docs/photos-api.md, "Export";
//! photo-editor contract §3).
//!
//! `POST /api/photos/export/ID`, body `{"format": "jpeg"|"png", "quality": 1..=100,
//! "max_side": null|16..=30000, "strip_location": true}` (every field optional; at most 16 KB).
//! Whoever may add files to the picture's folder (and see the picture). Answers 201 with
//! `{"id", "name", "folder", "path"}`: a new file "photo (edited).jpg", numbered when that name is
//! taken ("photo (edited) (2).jpg"), never over an existing file: it is saved through the same
//! path as an upload (store.rs `claim_name`: created with O_EXCL).
//!
//! The picture is decoded, turned upright (EXIF orientation), rendered with its saved edits and
//! encoded in a child process with the thumbnails' limits (decode.rs `photo-worker`), one export at
//! a time. Metadata is rebuilt from an allow-list ([`KEEP`]: camera, lens, exposure, dates,
//! author), never copied: no MakerNotes, no XMP, no thumbnail, no GPS unless `strip_location` is
//! false (then the EXIF GPS position only). The orientation is written as 1 (the pixels are
//! upright). The JPEG keeps the original's ICC colour profile; a PNG carries no metadata.

use std::collections::HashMap;
use std::io::{Cursor, Read};

use axum::body::Body;
use axum::http::Response;
use exif::{Field, In, Tag, Value};
use image::{DynamicImage, ImageEncoder, ImageFormat, RgbaImage};
use serde::{Deserialize, Serialize};
use serde_json::json;
use starbase_photo_edit::Edits;

use super::decode::{self, Job, Limits};
use super::{MAX_FILE, WORKER_EXE, media_of, service, sidecar};
use crate::{Shared, auth, blocking, error, send_json, sharing, store};

/// The most an export child may answer (a big PNG).
pub const MAX_OUTPUT: u64 = MAX_FILE;

/// One export at a time: each child may use the whole child memory budget.
static ONE_AT_A_TIME: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

/// How long a second export waits for the first before it is told to try again.
const SLOT_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Takes the export slot within `wait`, or None (the caller answers 429): a request must never sit
/// on one of the server's request slots for the length of someone else's export.
pub(super) async fn take_slot(sem: &'static tokio::sync::Semaphore, wait: std::time::Duration) -> Option<tokio::sync::SemaphorePermit<'static>> {
    tokio::time::timeout(wait, sem.acquire()).await.ok()?.ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Jpeg,
    Png,
}

impl Format {
    pub fn ext(self) -> &'static str {
        match self {
            Format::Jpeg => "jpg",
            Format::Png => "png",
        }
    }
}

fn jpeg() -> Format {
    Format::Jpeg
}
fn ninety() -> u8 {
    90
}
fn yes() -> bool {
    true
}

/// What an export makes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default = "jpeg")]
    pub format: Format,
    /// JPEG quality.
    #[serde(default = "ninety")]
    pub quality: u8,
    /// The longest side at most (never enlarged); None = full size.
    #[serde(default)]
    pub max_side: Option<u32>,
    /// Leave out where the picture was taken (EXIF GPS). On unless asked otherwise.
    #[serde(default = "yes")]
    pub strip_location: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { format: Format::Jpeg, quality: 90, max_side: None, strip_location: true }
    }
}

impl Options {
    pub fn check(&self) -> Result<(), String> {
        if !(1..=100).contains(&self.quality) {
            return Err("quality must be 1..=100".into());
        }
        if self.max_side.is_some_and(|m| !(16..=decode::LIMITS.max_side).contains(&m)) {
            return Err("max_side must be 16..=30000".into());
        }
        Ok(())
    }
}

// ------------------------------------------------------------------ in the child

/// EXIF fields copied from the original (the primary image's, of the kinds below, small).
const KEEP: [Tag; 23] = [
    Tag::Make,
    Tag::Model,
    Tag::DateTime,
    Tag::DateTimeOriginal,
    Tag::DateTimeDigitized,
    Tag::OffsetTime,
    Tag::OffsetTimeOriginal,
    Tag::OffsetTimeDigitized,
    Tag::SubSecTimeOriginal,
    Tag::ExposureTime,
    Tag::FNumber,
    Tag::ExposureProgram,
    Tag::PhotographicSensitivity,
    Tag::ExposureBiasValue,
    Tag::MeteringMode,
    Tag::Flash,
    Tag::FocalLength,
    Tag::FocalLengthIn35mmFilm,
    Tag::WhiteBalance,
    Tag::LensMake,
    Tag::LensModel,
    Tag::Artist,
    Tag::Copyright,
];
/// Also copied when the location is kept.
const LOCATION: [Tag; 7] =
    [Tag::GPSVersionID, Tag::GPSLatitudeRef, Tag::GPSLatitude, Tag::GPSLongitudeRef, Tag::GPSLongitude, Tag::GPSAltitudeRef, Tag::GPSAltitude];

/// A value small enough to copy, of a plain type.
fn small(v: &Value) -> bool {
    match v {
        Value::Ascii(s) => s.len() <= 2 && s.iter().map(Vec::len).sum::<usize>() <= 256,
        Value::Byte(x) => x.len() <= 8,
        Value::Short(x) => x.len() <= 8,
        Value::Long(x) => x.len() <= 8,
        Value::Rational(x) => x.len() <= 8,
        Value::SRational(x) => x.len() <= 8,
        _ => false,
    }
}

/// The new EXIF block (TIFF bytes): orientation 1 plus the allowed fields of the original.
fn exif_block(original: &[u8], keep_location: bool) -> Option<Vec<u8>> {
    let mut fields = vec![Field { tag: Tag::Orientation, ifd_num: In::PRIMARY, value: Value::Short(vec![1]) }];
    if let Ok(ex) = exif::Reader::new().read_from_container(&mut Cursor::new(original)) {
        for f in ex.fields() {
            let allowed = KEEP.contains(&f.tag) || (keep_location && LOCATION.contains(&f.tag));
            if f.ifd_num == In::PRIMARY && allowed && small(&f.value) && !fields.iter().any(|g| g.tag == f.tag) {
                fields.push(f.clone());
            }
        }
    }
    let mut w = exif::experimental::Writer::new();
    for f in &fields {
        w.push_field(f);
    }
    let mut out = Cursor::new(Vec::new());
    w.write(&mut out, false).ok()?;
    Some(out.into_inner())
}

/// `jpeg` with an EXIF (APP1) segment after its JFIF header.
fn with_exif(jpeg: Vec<u8>, tiff: &[u8]) -> Vec<u8> {
    let len = tiff.len() + 8;
    if len > usize::from(u16::MAX) || jpeg.len() < 4 {
        return jpeg;
    }
    // After SOI, and after APP0 (JFIF) when it comes first.
    let mut at = 2;
    if jpeg[2..4] == [0xFF, 0xE0] && jpeg.len() >= 6 {
        at = 4 + usize::from(u16::from_be_bytes([jpeg[4], jpeg[5]]));
    }
    if at > jpeg.len() {
        return jpeg;
    }
    let mut out = Vec::with_capacity(jpeg.len() + len + 2);
    out.extend_from_slice(&jpeg[..at]);
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&(len as u16).to_be_bytes());
    out.extend_from_slice(b"Exif\0\0");
    out.extend_from_slice(tiff);
    out.extend_from_slice(&jpeg[at..]);
    out
}

/// Shrinks `img` so its longest side is at most `max` (never enlarges).
fn shrink(img: RgbaImage, max: u32) -> Result<RgbaImage, String> {
    use fast_image_resize as fr;
    let (w, h) = img.dimensions();
    if w.max(h) <= max {
        return Ok(img);
    }
    let src = fr::images::Image::from_vec_u8(w, h, img.into_raw(), fr::PixelType::U8x4).map_err(|e| e.to_string())?;
    let out = decode::fit(src, max, fr::PixelType::U8x4)?;
    let (w, h) = (out.width(), out.height());
    RgbaImage::from_raw(w, h, out.into_vec()).ok_or_else(|| "wrong number of pixels".to_string())
}

/// The exported file's bytes: `bytes` decoded at full size, upright, rendered with `e`, shrunk
/// and encoded as `o` says, with the allowed metadata. Runs in the child (or, in tests, on the
/// calling thread).
pub fn export(bytes: &[u8], limits: &Limits, e: &Edits, o: &Options) -> Result<Vec<u8>, String> {
    o.check()?;
    let format = decode::sniff(bytes).ok_or("not a JPEG, PNG, WebP or GIF picture")?;
    let orientation = if format == ImageFormat::Gif { 1 } else { decode::exif_meta(bytes).orientation };
    let (w, h) = image::ImageReader::with_format(Cursor::new(bytes), format).into_dimensions().map_err(|e| format!("unreadable: {e}"))?;
    // Decoded whole, then about four copies of four bytes a pixel while rendering.
    let max = limits.max_pixels * 3 / 4;
    if w == 0 || h == 0 || w > limits.max_side || h > limits.max_side || u64::from(w) * u64::from(h) > max {
        return Err(format!("too big to export ({w} × {h} pixels)"));
    }
    let (img, icc) = if format == ImageFormat::Jpeg {
        decode::jpeg_scaled(bytes, u32::from(u16::MAX), limits)?
    } else {
        let mut r = image::ImageReader::with_format(Cursor::new(bytes), format);
        let mut l = image::Limits::default();
        l.max_image_width = Some(limits.max_side);
        l.max_image_height = Some(limits.max_side);
        l.max_alloc = Some(limits.max_alloc);
        r.limits(l);
        (r.decode().map_err(|e| format!("unreadable: {e}"))?, None)
    };
    let alpha = img.color().has_alpha();
    let up = decode::upright(img, orientation)?;
    let mut out = starbase_photo_edit::render(&up, e);
    drop(up);
    if let Some(m) = o.max_side {
        out = shrink(out, m)?;
    }
    let (w, h) = out.dimensions();
    match o.format {
        Format::Png => {
            if alpha {
                decode::encode_png(out.as_raw(), w, h)
            } else {
                let rgb = DynamicImage::ImageRgba8(out).into_rgb8();
                let mut buf = Vec::new();
                image::codecs::png::PngEncoder::new(&mut buf)
                    .write_image(rgb.as_raw(), w, h, image::ExtendedColorType::Rgb8)
                    .map_err(|e| e.to_string())?;
                Ok(buf)
            }
        }
        Format::Jpeg => {
            // JPEG has no transparency: transparent parts become white.
            let mut rgb = Vec::with_capacity(w as usize * h as usize * 3);
            for p in out.as_raw().as_chunks::<4>().0 {
                let a = u16::from(p[3]);
                for c in &p[..3] {
                    rgb.push(((u16::from(*c) * a + 255 * (255 - a) + 127) / 255) as u8);
                }
            }
            drop(out);
            let mut buf = Vec::new();
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, o.quality);
            if let Some(icc) = icc {
                // A profile the encoder can't carry is left out (the colours are then sRGB's).
                let _ = enc.set_icc_profile(icc);
            }
            enc.write_image(&rgb, w, h, image::ExtendedColorType::Rgb8).map_err(|e| e.to_string())?;
            Ok(match exif_block(bytes, !o.strip_location) {
                Some(tiff) => with_exif(buf, &tiff),
                None => buf,
            })
        }
    }
}

/// [`export`] on this thread, with a panic turned into an error.
pub fn export_guarded(bytes: &[u8], limits: &Limits, e: &Edits, o: &Options) -> Result<Vec<u8>, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| export(bytes, limits, e, o)))
        .unwrap_or_else(|_| Err("the decoder failed (panic)".into()))
}

// ------------------------------------------------------------------ the route

/// "photo.jpg" → "photo (edited).png" for a PNG export.
fn exported_name(name: &str, f: Format) -> String {
    let stem = match name.rsplit_once('.') {
        Some((s, _)) if !s.is_empty() => s,
        _ => name,
    };
    format!("{stem} (edited).{}", f.ext())
}

pub async fn route(app: &Shared, m: &str, id: String, me: Option<auth::User>, body: &[u8]) -> Response<Body> {
    if m != "POST" {
        return error(405, "use POST");
    }
    if body.len() > super::edits::MAX_BODY {
        return error(413, "request too large");
    }
    let opts: Options = if body.iter().all(u8::is_ascii_whitespace) {
        Options::default()
    } else {
        match serde_json::from_slice(body) {
            Ok(o) => o,
            Err(e) => return error(400, &format!("These aren't export settings: {e}")),
        }
    };
    if let Err(e) = opts.check() {
        return error(400, &e);
    }
    // 1. Under the store lock: who may do it, the picture (opened once, without links), its edits.
    let me2 = me.clone();
    let first = blocking(app, move |a| -> Result<(String, std::fs::File), Response<Body>> {
        let acc = sharing::Access::new(&a.store, a.auth, me2.as_ref());
        if acc.file_role(&id) < sharing::Role::View {
            return Err(error(403, "You don't have access to that."));
        }
        let path = acc.file_paths.get(&id).cloned().ok_or_else(|| error(404, "no such file"))?;
        match media_of(&path) {
            None => return Err(error(404, "not a picture")),
            Some((_, false)) => return Err(error(409, "Starbase can't export this kind of picture yet.")),
            Some(_) => {}
        }
        if acc.folder_role(&store::parent_of(&path)) < sharing::Role::Edit {
            return Err(error(403, "You can see this picture but can't add files to its folder."));
        }
        let file = a.store.open_safe(&path).ok_or_else(|| error(404, "no such file"))?;
        if file.metadata().map(|m| m.len()).unwrap_or(u64::MAX) > MAX_FILE {
            return Err(error(413, "The picture is too big to export."));
        }
        Ok((path, file))
    })
    .await;
    let (path, file) = match first {
        Some(Ok(f)) => f,
        Some(Err(r)) => return r,
        None => return error(500, "server error"),
    };
    // 2. Without the lock: the saved edits (parsed here: untrusted XML), then the export.
    let svc = service(&app.store);
    let memory = svc.as_ref().map_or(decode::CHILD_MEMORY, |s| s.cfg.child_memory);
    let Some(permit) = take_slot(&ONE_AT_A_TIME, SLOT_WAIT).await else {
        return error(429, "Another export is running; try again in a moment.");
    };
    let (app2, path2, opts2) = (app.clone(), path.clone(), opts.clone());
    let made = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
        let _one = permit;
        use std::io::{Seek, SeekFrom};
        let edits = saved_edits(&app2.store, &path2, &file);
        // Its first bytes were read for its size: from the start again.
        let mut file = file;
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        match WORKER_EXE.get() {
            Some(exe) => decode::run_export(exe, file, memory, &Job { edits: Some(edits), export: Some(opts2) }),
            None => {
                let mut bytes = Vec::new();
                (&file).take(MAX_FILE).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
                export_guarded(&bytes, &Limits::for_memory(memory), &edits, &opts2)
            }
        }
    })
    .await;
    let bytes = match made {
        Ok(Ok(b)) => b,
        Ok(Err(why)) => return send_json(&json!({"error": format!("The picture couldn't be exported: {why}")}), 422),
        Err(_) => return error(500, "server error"),
    };
    // 3. Under the lock again: still allowed, then saved as a new file, as an upload is.
    let folder = store::parent_of(&path);
    let name = exported_name(&store::base_of(&path), opts.format);
    let who = me.as_ref().map(|u| u.username.clone());
    let saved = blocking(app, move |a| {
        let acc = sharing::Access::new(&a.store, a.auth, me.as_ref());
        if acc.folder_role(&folder) < sharing::Role::Edit {
            return Err(error(403, "You can no longer add files to this folder."));
        }
        let q: HashMap<String, String> =
            [("folder", folder.as_str()), ("name", name.as_str()), ("as_is", "1")].into_iter().map(|(k, v)| (k.into(), v.into())).collect();
        match a.store.route("POST", &["files".to_string()], &q, None, &bytes) {
            Ok(store::Reply::Json(v, _)) => Ok(v),
            Ok(_) => Err(error(500, "server error")),
            Err(b) => Err(error(b.code, &b.msg)),
        }
    })
    .await;
    let v = match saved {
        Some(Ok(v)) => v,
        Some(Err(r)) => return r,
        None => return error(500, "server error"),
    };
    let (folder, name) = (v["folder"].as_str().unwrap_or_default().to_string(), v["name"].as_str().unwrap_or_default().to_string());
    let new_path = store::join(&folder, &name);
    // Who made the new file and when (Details), as an upload records it.
    if let (Some(who), Some(id)) = (who, v["id"].as_str().map(str::to_string)) {
        blocking(app, move |a| crate::spaces::note_changed(&a.store, &id, &who)).await;
    }
    if let Some(svc) = svc
        && svc.covers(&new_path)
    {
        svc.enqueue(vec![new_path.clone()], true);
    }
    send_json(&json!({"id": v["id"], "name": name, "folder": folder, "path": new_path}), 201)
}

/// The picture's saved edits (none when its sidecar is missing or unreadable).
fn saved_edits(store: &store::Store, path: &str, file: &std::fs::File) -> Edits {
    let Some(dir) = sidecar::Folder::open(store, &store::parent_of(path)) else { return Edits::default() };
    let Ok(Some(d)) = dir.read(&sidecar::name_for(&store::base_of(path))) else { return Edits::default() };
    let geo = super::edits::geo_of(Some(file));
    sidecar::read_for(&d.bytes, geo.as_ref()).map(|p| p.edits).unwrap_or_default()
}
