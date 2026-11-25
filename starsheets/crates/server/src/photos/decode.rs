//! Turning one picture's bytes into its index entry (EXIF) and its thumbnails, with limits.
//!
//! The bytes are untrusted: anyone who can put a file in a folder (a phone syncing over a file
//! share, someone a folder is shared with) chooses them. So:
//! - the format comes from the first bytes, not the name, and only JPEG, PNG, WebP and GIF are
//!   decoded (GIF and WebP: the first frame);
//! - width and height are read from the header first, and a picture over [`Limits`] is refused
//!   before anything is decoded (a "decompression bomb": a tiny file that unpacks to gigabytes);
//! - the decoders get a memory limit of their own (`image::Limits`);
//! - in the server, all of this runs in a child process ([`run_isolated`]: `starsheets-server
//!   photo-worker`) with an address-space and CPU-time limit, killed after a wall-clock timeout;
//!   a crash or panic there is one failed picture, never the server. Without the server's own
//!   program at hand (unit tests) it runs on the calling thread inside `catch_unwind`.
//!
//! Thumbnails: the longest side at most each of [`SIZES`] (never enlarged), EXIF orientation
//! applied, JPEG quality 80 (PNG when the picture has transparency).

use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::time::Duration;

use fast_image_resize as fr;
use image::{DynamicImage, ImageFormat};
use serde::{Deserialize, Serialize};

/// The thumbnail sizes made for every picture (longest side, pixels): the grid and the viewer.
pub const SIZES: [u32; 2] = [256, 1024];
/// Goes up whenever thumbnails would come out differently (sizes, quality, orientation rules),
/// so the ones already made are made again.
pub const RENDER: &str = "r1";
/// JPEG quality of thumbnails (Immich uses 80 too).
const QUALITY: u8 = 80;

/// What a picture may be, at most, before it is decoded.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Width × height. JPEGs are decoded scaled down (1/2 to 1/8), so they may be bigger.
    pub max_pixels: u64,
    pub max_pixels_jpeg: u64,
    /// Either side.
    pub max_side: u32,
    /// Memory one decoder may ask for (image::Limits::max_alloc).
    pub max_alloc: u64,
}

pub const LIMITS: Limits = Limits { max_pixels: 50_000_000, max_pixels_jpeg: 150_000_000, max_side: 30_000, max_alloc: 512 * 1024 * 1024 };

/// The child's own limits: address space (bytes) and CPU time (seconds).
pub const CHILD_MEMORY: u64 = 1536 * 1024 * 1024;
pub const CHILD_CPU_SECS: u64 = 30;
/// How long a child may take, start to end (it runs at idle I/O priority, so allow for waiting).
pub const CHILD_TIMEOUT: Duration = Duration::from_secs(60);
/// The most a child may answer with (two thumbnails and a little JSON).
const MAX_ANSWER: u64 = 32 * 1024 * 1024;

/// What the index keeps about a picture (from its header and EXIF).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    /// As shown: orientation applied (a portrait photo stored sideways is taller than wide).
    pub width: u32,
    pub height: u32,
    /// EXIF orientation (1-8; 1 = as stored).
    pub orientation: u16,
    /// When it was taken (EXIF DateTimeOriginal, else DateTimeDigitized, else DateTime), as
    /// `YYYY-MM-DDTHH:MM:SS`, with `+HH:MM` when the camera wrote its time zone.
    pub taken: Option<String>,
    pub make: Option<String>,
    pub model: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub alt: Option<f64>,
}

#[derive(Debug)]
pub struct Thumb {
    pub size: u32,
    /// "jpg" or "png".
    pub ext: &'static str,
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
pub struct Made {
    pub meta: Meta,
    pub thumbs: Vec<Thumb>,
}

/// The formats decoded, from the file's first bytes.
pub fn sniff(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(ImageFormat::Gif)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(ImageFormat::WebP)
    } else {
        None
    }
}

/// Reads the picture and makes its thumbnails (on this thread: see [`make_guarded`] and
/// [`run_isolated`] for the ways it is called).
pub fn make(bytes: &[u8], limits: &Limits) -> Result<Made, String> {
    let format = sniff(bytes).ok_or("not a JPEG, PNG, WebP or GIF picture")?;
    let mut meta = if format == ImageFormat::Gif { Meta::default() } else { exif_meta(bytes) };
    let (w, h) = image::ImageReader::with_format(Cursor::new(bytes), format).into_dimensions().map_err(|e| format!("unreadable: {e}"))?;
    if w == 0 || h == 0 {
        return Err("the picture has no pixels".into());
    }
    let max = if format == ImageFormat::Jpeg { limits.max_pixels_jpeg } else { limits.max_pixels };
    if w > limits.max_side || h > limits.max_side || u64::from(w) * u64::from(h) > max {
        return Err(format!("too big to make thumbnails of ({w} × {h} pixels)"));
    }
    let o = meta.orientation;
    (meta.width, meta.height) = if (5..=8).contains(&o) { (h, w) } else { (w, h) };
    let largest = SIZES.iter().copied().max().unwrap_or(1024);
    let img = if format == ImageFormat::Jpeg {
        jpeg_scaled(bytes, largest, limits)?
    } else {
        let mut r = image::ImageReader::with_format(Cursor::new(bytes), format);
        let mut l = image::Limits::default();
        l.max_image_width = Some(limits.max_side);
        l.max_image_height = Some(limits.max_side);
        l.max_alloc = Some(limits.max_alloc);
        r.limits(l);
        r.decode().map_err(|e| format!("unreadable: {e}"))?
    };
    let alpha = img.color().has_alpha();
    let (mut src, kind) = if alpha {
        let i = img.into_rgba8();
        let (w, h) = i.dimensions();
        (fr::images::Image::from_vec_u8(w, h, i.into_raw(), fr::PixelType::U8x4).map_err(|e| e.to_string())?, fr::PixelType::U8x4)
    } else {
        let i = img.into_rgb8();
        let (w, h) = i.dimensions();
        (fr::images::Image::from_vec_u8(w, h, i.into_raw(), fr::PixelType::U8x3).map_err(|e| e.to_string())?, fr::PixelType::U8x3)
    };
    // Largest first, each smaller one from the one before (less work, same look).
    let mut sizes = SIZES.to_vec();
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    let mut thumbs = Vec::new();
    for size in sizes {
        src = fit(src, size, kind)?;
        let (buf, tw, th) = orient(src.buffer(), src.width(), src.height(), if alpha { 4 } else { 3 }, o);
        let bytes = if alpha { encode_png(&buf, tw, th)? } else { encode_jpeg(&buf, tw, th)? };
        thumbs.push(Thumb { size, ext: if alpha { "png" } else { "jpg" }, bytes });
    }
    thumbs.sort_by_key(|t| t.size);
    Ok(Made { meta, thumbs })
}

/// [`make`] on this thread, with a panic in a decoder turned into an error.
pub fn make_guarded(bytes: &[u8], limits: &Limits) -> Result<Made, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| make(bytes, limits))).unwrap_or_else(|_| Err("the decoder failed (panic)".into()))
}

/// A JPEG decoded at 1/8, 1/4 or 1/2 of its size when that is still at least `want` pixels on
/// its longest side: much less work for a thumbnail of a big photo.
fn jpeg_scaled(bytes: &[u8], want: u32, limits: &Limits) -> Result<DynamicImage, String> {
    let mut d = jpeg_decoder::Decoder::new(Cursor::new(bytes));
    d.set_max_decoding_buffer_size(limits.max_alloc as usize);
    d.read_info().map_err(|e| format!("unreadable: {e}"))?;
    let want16 = want.min(u32::from(u16::MAX)) as u16;
    d.scale(want16, want16).map_err(|e| format!("unreadable: {e}"))?;
    let px = d.decode().map_err(|e| format!("unreadable: {e}"))?;
    let info = d.info().ok_or("unreadable: no header")?;
    let (w, h) = (u32::from(info.width), u32::from(info.height));
    let bad = || "unreadable: wrong number of pixels".to_string();
    match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => image::RgbImage::from_raw(w, h, px).map(DynamicImage::ImageRgb8).ok_or_else(bad),
        jpeg_decoder::PixelFormat::L8 => image::GrayImage::from_raw(w, h, px).map(DynamicImage::ImageLuma8).ok_or_else(bad),
        jpeg_decoder::PixelFormat::L16 => {
            let px: Vec<u16> = px.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
            image::ImageBuffer::<image::Luma<u16>, _>::from_raw(w, h, px).map(DynamicImage::ImageLuma16).ok_or_else(bad)
        }
        jpeg_decoder::PixelFormat::CMYK32 => {
            // Adobe's CMYK JPEGs store the channels inverted; this is the usual approximation.
            let rgb: Vec<u8> = px
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|c| {
                    let k = u16::from(c[3]);
                    [(u16::from(c[0]) * k / 255) as u8, (u16::from(c[1]) * k / 255) as u8, (u16::from(c[2]) * k / 255) as u8]
                })
                .collect();
            image::RgbImage::from_raw(w, h, rgb).map(DynamicImage::ImageRgb8).ok_or_else(bad)
        }
    }
}

/// Scales `src` down so its longest side is at most `long` (never up).
fn fit(src: fr::images::Image<'static>, long: u32, kind: fr::PixelType) -> Result<fr::images::Image<'static>, String> {
    let (w, h) = (src.width(), src.height());
    if w.max(h) <= long {
        return Ok(src);
    }
    let (nw, nh) = if w >= h {
        (long, ((u64::from(h) * u64::from(long) + u64::from(w) / 2) / u64::from(w)).max(1) as u32)
    } else {
        (((u64::from(w) * u64::from(long) + u64::from(h) / 2) / u64::from(h)).max(1) as u32, long)
    };
    let mut dst = fr::images::Image::new(nw, nh, kind);
    let opts = fr::ResizeOptions::new().resize_alg(fr::ResizeAlg::Convolution(fr::FilterType::CatmullRom));
    fr::Resizer::new().resize(&src, &mut dst, &opts).map_err(|e| e.to_string())?;
    Ok(dst)
}

/// Turns a stored picture (`c` bytes per pixel) the way EXIF orientation `o` says it is shown.
pub fn orient(buf: &[u8], w: u32, h: u32, c: usize, o: u16) -> (Vec<u8>, u32, u32) {
    if !(2..=8).contains(&o) {
        return (buf.to_vec(), w, h);
    }
    let (w, h) = (w as usize, h as usize);
    let swap = o >= 5;
    let (nw, nh) = if swap { (h, w) } else { (w, h) };
    let mut out = vec![0u8; buf.len()];
    for y in 0..h {
        for x in 0..w {
            // Where pixel (x, y) of the stored picture goes.
            let (nx, ny) = match o {
                2 => (w - 1 - x, y),         // mirrored
                3 => (w - 1 - x, h - 1 - y), // upside down
                4 => (x, h - 1 - y),         // mirrored upside down
                5 => (y, x),                 // transposed
                6 => (h - 1 - y, x),         // turned a quarter clockwise
                7 => (h - 1 - y, w - 1 - x), // transversed
                _ => (y, w - 1 - x),         // 8: turned a quarter anticlockwise
            };
            let (from, to) = ((y * w + x) * c, (ny * nw + nx) * c);
            out[to..to + c].copy_from_slice(&buf[from..from + c]);
        }
    }
    (out, nw as u32, nh as u32)
}

fn encode_jpeg(rgb: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, QUALITY)
        .encode(rgb, w, h, image::ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

fn encode_png(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(&mut out, image::codecs::png::CompressionType::Fast, image::codecs::png::FilterType::Adaptive)
        .write_image(rgba, w, h, image::ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

// ---------------------------------------------------------------- EXIF

/// Orientation, date, camera and place from the picture's EXIF (all optional; none = defaults).
pub fn exif_meta(bytes: &[u8]) -> Meta {
    use exif::{In, Tag, Value};
    let mut m = Meta { orientation: 1, ..Meta::default() };
    let Ok(e) = exif::Reader::new().read_from_container(&mut Cursor::new(bytes)) else { return m };
    let field = |t: Tag| e.get_field(t, In::PRIMARY);
    let text = |t: Tag| -> Option<String> {
        match &field(t)?.value {
            Value::Ascii(v) => {
                let s = String::from_utf8_lossy(v.first()?).trim().trim_end_matches('\0').trim().to_string();
                (!s.is_empty()).then(|| s.chars().take(120).collect())
            }
            _ => None,
        }
    };
    if let Some(o) = field(Tag::Orientation).and_then(|f| f.value.get_uint(0))
        && (1..=8).contains(&o)
    {
        m.orientation = o as u16;
    }
    let when =
        [(Tag::DateTimeOriginal, Tag::OffsetTimeOriginal), (Tag::DateTimeDigitized, Tag::OffsetTimeDigitized), (Tag::DateTime, Tag::OffsetTime)];
    m.taken = when.iter().find_map(|(t, zone)| {
        let d = exif::DateTime::from_ascii(text(*t)?.as_bytes()).ok()?;
        if d.year < 1800 || d.month == 0 || d.month > 12 || d.day == 0 || d.day > 31 || d.hour > 23 || d.minute > 59 || d.second > 60 {
            return None;
        }
        let mut s = format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", d.year, d.month, d.day, d.hour, d.minute, d.second.min(59));
        if let Some(z) = text(*zone).filter(|z| z.len() == 6 && (z.starts_with('+') || z.starts_with('-')) && z.as_bytes()[3] == b':') {
            s.push_str(&z);
        }
        Some(s)
    });
    m.make = text(Tag::Make);
    m.model = text(Tag::Model);
    let degrees = |t: Tag, r: Tag, neg: &str| -> Option<f64> {
        let Value::Rational(v) = &field(t)?.value else { return None };
        if v.len() < 3 || v.iter().any(|x| x.denom == 0) {
            return None;
        }
        let d = v[0].to_f64() + v[1].to_f64() / 60.0 + v[2].to_f64() / 3600.0;
        let sign = if text(r).is_some_and(|s| s.eq_ignore_ascii_case(neg)) { -1.0 } else { 1.0 };
        Some(d * sign)
    };
    let lat = degrees(Tag::GPSLatitude, Tag::GPSLatitudeRef, "S").filter(|v| v.abs() <= 90.0);
    let lon = degrees(Tag::GPSLongitude, Tag::GPSLongitudeRef, "W").filter(|v| v.abs() <= 180.0);
    if let (Some(lat), Some(lon)) = (lat, lon) {
        m.lat = Some(lat);
        m.lon = Some(lon);
        m.alt = match &field(Tag::GPSAltitude).map(|f| &f.value) {
            Some(Value::Rational(v)) if !v.is_empty() && v[0].denom != 0 => {
                let below = matches!(field(Tag::GPSAltitudeRef).and_then(|f| f.value.get_uint(0)), Some(1));
                Some(if below { -v[0].to_f64() } else { v[0].to_f64() })
            }
            _ => None,
        };
    }
    m
}

// ---------------------------------------------------------------- the child process

/// Header of a child's answer (one JSON line, then the thumbnails' bytes one after another).
#[derive(Serialize, Deserialize)]
struct Answer {
    ok: bool,
    #[serde(default)]
    error: String,
    #[serde(default)]
    meta: Meta,
    /// (size, "jpg"/"png", byte length)
    #[serde(default)]
    thumbs: Vec<(u32, String, u64)>,
}

/// `starsheets-server photo-worker`: reads one picture from stdin, writes the answer to stdout.
/// Runs before anything else in the program starts (no async runtime, no threads).
pub fn child_main() -> ! {
    // SAFETY: plain system calls on this process's own limits and priority.
    unsafe {
        let cap = |res, v: u64| {
            let l = libc::rlimit { rlim_cur: v as libc::rlim_t, rlim_max: v as libc::rlim_t };
            libc::setrlimit(res, &l);
        };
        cap(libc::RLIMIT_AS, CHILD_MEMORY);
        cap(libc::RLIMIT_CPU, CHILD_CPU_SECS);
        cap(libc::RLIMIT_CORE, 0);
        // Never more than a few files open, and nothing written to disk.
        cap(libc::RLIMIT_FSIZE, 0);
        libc::setpriority(libc::PRIO_PROCESS, 0, 15);
    }
    let mut bytes = Vec::new();
    if std::io::stdin().lock().take(super::MAX_FILE + 1).read_to_end(&mut bytes).is_err() || bytes.len() as u64 > super::MAX_FILE {
        std::process::exit(2);
    }
    let out = std::io::stdout();
    let mut out = out.lock();
    let r = make(&bytes, &LIMITS);
    drop(bytes);
    let (answer, data) = match r {
        Ok(m) => {
            let list = m.thumbs.iter().map(|t| (t.size, t.ext.to_string(), t.bytes.len() as u64)).collect();
            (Answer { ok: true, error: String::new(), meta: m.meta, thumbs: list }, m.thumbs.into_iter().map(|t| t.bytes).collect::<Vec<_>>())
        }
        Err(e) => (Answer { ok: false, error: e, meta: Meta::default(), thumbs: vec![] }, vec![]),
    };
    let mut line = serde_json::to_vec(&answer).unwrap_or_default();
    line.push(b'\n');
    let ok = out.write_all(&line).is_ok() && data.iter().all(|d| out.write_all(d).is_ok()) && out.flush().is_ok();
    std::process::exit(if ok { 0 } else { 3 })
}

/// Runs [`make`] for `bytes` in a child process (`exe photo-worker`), within its limits.
pub fn run_isolated(exe: &Path, bytes: &[u8]) -> Result<Made, String> {
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("photo-worker");
    run_child(cmd, bytes, CHILD_TIMEOUT)
}

/// Sends `bytes` to the child `cmd` and reads its answer, killing it after `timeout`.
pub fn run_child(mut cmd: std::process::Command, bytes: &[u8], timeout: Duration) -> Result<Made, String> {
    use std::process::Stdio;
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_clear()
        .spawn()
        .map_err(|e| format!("couldn't start the picture reader: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("no pipe")?;
    let (tx, rx) = std::sync::mpsc::channel();
    // Read on a thread of its own, so a child that never ends (or never reads) can be killed.
    std::thread::spawn(move || {
        let mut answer = Vec::new();
        let r = (&mut stdout).take(MAX_ANSWER).read_to_end(&mut answer);
        let _ = tx.send(r.map(|_| answer));
    });
    let mut stdin = child.stdin.take().ok_or("no pipe")?;
    let wrote = stdin.write_all(bytes);
    drop(stdin);
    let got = rx.recv_timeout(timeout);
    if got.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("took over {} s: stopped", timeout.as_secs()));
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let answer = match got {
        Ok(Ok(a)) => a,
        _ => return Err("couldn't read the picture reader's answer".into()),
    };
    if !status.success() {
        use std::os::unix::process::ExitStatusExt;
        return Err(match status.signal() {
            Some(s) => format!("the picture reader stopped (signal {s}: out of memory or time, or a crash)"),
            None if wrote.is_err() => "the picture reader stopped early".into(),
            None => format!("the picture reader failed (exit {})", status.code().unwrap_or(-1)),
        });
    }
    let nl = answer.iter().position(|&b| b == b'\n').ok_or("the picture reader's answer was cut short")?;
    let head: Answer = serde_json::from_slice(&answer[..nl]).map_err(|_| "the picture reader's answer made no sense")?;
    if !head.ok {
        return Err(head.error);
    }
    let mut rest = &answer[nl + 1..];
    let mut thumbs = Vec::new();
    for (size, ext, len) in head.thumbs {
        let len = len as usize;
        if rest.len() < len || !SIZES.contains(&size) {
            return Err("the picture reader's answer was cut short".into());
        }
        let ext = if ext == "png" { "png" } else { "jpg" };
        thumbs.push(Thumb { size, ext, bytes: rest[..len].to_vec() });
        rest = &rest[len..];
    }
    Ok(Made { meta: head.meta, thumbs })
}
