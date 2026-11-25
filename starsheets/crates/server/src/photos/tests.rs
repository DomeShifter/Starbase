//! Photos: decoding with limits, orientation, EXIF, the service's index and thumbnails, and the
//! routes' access checks. Pictures are made here, a few pixels each.

use std::collections::HashMap;
use std::io::Cursor;
use std::time::{Duration, Instant};

use super::decode::{self, LIMITS, Meta};
use super::index::State;
use super::*;

fn tmp(what: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("starbase-photos-{what}-{}", crate::store::random_hex(6)));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A picture `w` × `h`: the left half red, the right half blue.
fn halves(w: u32, h: u32) -> image::RgbImage {
    image::RgbImage::from_fn(w, h, |x, _| if x < w / 2 { image::Rgb([255, 0, 0]) } else { image::Rgb([0, 0, 255]) })
}

fn jpeg(img: &image::RgbImage) -> Vec<u8> {
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 95).encode_image(img).unwrap();
    out
}

fn encoded(img: &image::DynamicImage, f: image::ImageFormat) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, f).unwrap();
    out.into_inner()
}

/// A big-endian TIFF/EXIF block with orientation, camera, date (in the EXIF sub-IFD) and GPS.
/// Degrees, minutes, seconds.
type Dms = (u32, u32, u32);

fn exif_block(orientation: u16, date: &str, gps: Option<(Dms, char, Dms, char)>) -> Vec<u8> {
    // Entries: (tag, type, count, value bytes); values over 4 bytes go after the IFDs.
    type Entry = (u16, u16, u32, Vec<u8>);
    fn ifd(entries: &[Entry], start: u32, data: &mut Vec<u8>, data_at: u32) -> Vec<u8> {
        let mut out = (entries.len() as u16).to_be_bytes().to_vec();
        for (tag, ty, count, val) in entries {
            out.extend(tag.to_be_bytes());
            out.extend(ty.to_be_bytes());
            out.extend(count.to_be_bytes());
            if val.len() <= 4 {
                let mut v = val.clone();
                v.resize(4, 0);
                out.extend(v);
            } else {
                out.extend((data_at + data.len() as u32).to_be_bytes());
                data.extend(val);
                if data.len() % 2 == 1 {
                    data.push(0);
                }
            }
        }
        out.extend(0u32.to_be_bytes());
        let _ = start;
        out
    }
    let ascii = |s: &str| {
        let mut v = s.as_bytes().to_vec();
        v.push(0);
        v
    };
    let rat = |(a, b, c): (u32, u32, u32)| [a, 1, b, 1, c, 1].iter().flat_map(|n| n.to_be_bytes()).collect::<Vec<u8>>();
    let size = |e: &[Entry]| 2 + 12 * e.len() as u32 + 4;
    // Layout: header(8) | IFD0 | ExifIFD | GPS IFD | data
    let n0 = 5 + u32::from(gps.is_some());
    let ifd0_at = 8;
    let exif_at = ifd0_at + 2 + 12 * n0 + 4;
    let exif_entries: Vec<Entry> = vec![(0x9003, 2, ascii(date).len() as u32, ascii(date)), (0x9011, 2, 7, ascii("+01:00"))];
    let gps_at = exif_at + size(&exif_entries);
    let gps_entries: Vec<Entry> = match gps {
        Some((lat, ns, lon, ew)) => {
            vec![(0x0001, 2, 2, ascii(&ns.to_string())), (0x0002, 5, 3, rat(lat)), (0x0003, 2, 2, ascii(&ew.to_string())), (0x0004, 5, 3, rat(lon))]
        }
        None => vec![],
    };
    let data_at = gps_at + if gps.is_some() { size(&gps_entries) } else { 0 };
    let mut ifd0: Vec<Entry> = vec![
        (0x010F, 2, 6, ascii("Canon")),
        (0x0110, 2, 9, ascii("EOS R6 2")),
        (0x0112, 3, 1, orientation.to_be_bytes().to_vec()),
        (0x0132, 2, 20, ascii("2000:01:01 00:00:00")),
        (0x8769, 4, 1, exif_at.to_be_bytes().to_vec()),
    ];
    if gps.is_some() {
        ifd0.push((0x8825, 4, 1, gps_at.to_be_bytes().to_vec()));
    }
    let mut data = Vec::new();
    let mut out = b"MM\x00\x2a\x00\x00\x00\x08".to_vec();
    out.extend(ifd(&ifd0, ifd0_at, &mut data, data_at));
    out.extend(ifd(&exif_entries, exif_at, &mut data, data_at));
    if gps.is_some() {
        out.extend(ifd(&gps_entries, gps_at, &mut data, data_at));
    }
    assert_eq!(out.len() as u32, data_at);
    out.extend(data);
    out
}

/// `jpeg` with an EXIF (APP1) segment put right after its start marker.
fn with_exif(jpeg: &[u8], tiff: &[u8]) -> Vec<u8> {
    let mut seg = b"Exif\x00\x00".to_vec();
    seg.extend(tiff);
    let mut out = jpeg[..2].to_vec();
    out.extend([0xFF, 0xE1]);
    out.extend(((seg.len() + 2) as u16).to_be_bytes());
    out.extend(seg);
    out.extend(&jpeg[2..]);
    out
}

fn decoded(t: &decode::Thumb) -> image::RgbImage {
    image::load_from_memory(&t.bytes).unwrap().into_rgb8()
}

#[test]
fn thumbnails_are_made_small_and_never_enlarged() {
    let made = decode::make(&jpeg(&halves(3000, 2000)), &LIMITS).unwrap();
    let sizes: Vec<(u32, u32, u32)> = made.thumbs.iter().map(|t| (t.size, decoded(t).width(), decoded(t).height())).collect();
    assert_eq!(sizes, vec![(256, 256, 171), (1024, 1024, 683)]);
    assert!(made.thumbs.iter().all(|t| t.ext == "jpg" && t.bytes.starts_with(&[0xFF, 0xD8])));
    assert_eq!((made.meta.width, made.meta.height, made.meta.orientation), (3000, 2000, 1));
    let small = decode::make(&jpeg(&halves(40, 20)), &LIMITS).unwrap();
    assert!(small.thumbs.iter().all(|t| decoded(t).dimensions() == (40, 20)), "a small picture stays its size");
}

#[test]
fn every_format_and_transparency() {
    let rgb = image::DynamicImage::ImageRgb8(halves(64, 32));
    for f in [image::ImageFormat::Png, image::ImageFormat::Gif, image::ImageFormat::WebP] {
        let made = decode::make(&encoded(&rgb, f), &LIMITS).unwrap_or_else(|e| panic!("{f:?}: {e}"));
        assert_eq!((made.meta.width, made.meta.height), (64, 32), "{f:?}");
        let px = decoded(&made.thumbs[0]);
        assert!(px.get_pixel(2, 16)[0] > 200 && px.get_pixel(60, 16)[2] > 200, "{f:?}: red left, blue right");
    }
    let clear = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(10, 10, image::Rgba([0, 255, 0, 0])));
    let made = decode::make(&encoded(&clear, image::ImageFormat::Png), &LIMITS).unwrap();
    assert!(made.thumbs.iter().all(|t| t.ext == "png"), "transparency kept: PNG thumbnails");
    // The format comes from the bytes: a PNG named .jpg is still read; text is refused.
    assert!(decode::make(b"<svg xmlns='http://www.w3.org/2000/svg'/>", &LIMITS).unwrap_err().contains("not a JPEG"));
    assert!(decode::make_guarded(b"\xFF\xD8\xFF\xE0 garbage that is not a jpeg at all", &LIMITS, None).is_err());
}

#[test]
fn orientation_is_applied() {
    // Stored 40 × 20 (red left, blue right); EXIF 6 = turn a quarter clockwise to show.
    let pic = with_exif(&jpeg(&halves(40, 20)), &exif_block(6, "2024:07:14 18:30:05", None));
    let made = decode::make(&pic, &LIMITS).unwrap();
    assert_eq!((made.meta.width, made.meta.height, made.meta.orientation), (20, 40, 6));
    let t = decoded(&made.thumbs[0]);
    assert_eq!(t.dimensions(), (20, 40));
    assert!(t.get_pixel(10, 3)[0] > 200 && t.get_pixel(10, 3)[2] < 60, "the left side is now at the top: {:?}", t.get_pixel(10, 3));
    assert!(t.get_pixel(10, 36)[2] > 200, "the right side at the bottom");
    // All eight, on a 3 × 2 picture of numbers.
    let buf: Vec<u8> = (1..=6).collect();
    let want: [(&[u8], u32, u32); 8] = [
        (&[1, 2, 3, 4, 5, 6], 3, 2),
        (&[3, 2, 1, 6, 5, 4], 3, 2),
        (&[6, 5, 4, 3, 2, 1], 3, 2),
        (&[4, 5, 6, 1, 2, 3], 3, 2),
        (&[1, 4, 2, 5, 3, 6], 2, 3),
        (&[4, 1, 5, 2, 6, 3], 2, 3),
        (&[6, 3, 5, 2, 4, 1], 2, 3),
        (&[3, 6, 2, 5, 1, 4], 2, 3),
    ];
    for (o, (px, w, h)) in want.iter().enumerate() {
        assert_eq!(decode::orient(&buf, 3, 2, 1, o as u16 + 1), (px.to_vec(), *w, *h), "orientation {}", o + 1);
    }
}

#[test]
fn exif_date_camera_and_place() {
    let pic = with_exif(&jpeg(&halves(8, 8)), &exif_block(1, "2024:07:14 18:30:05", Some(((51, 30, 36), 'N', (0, 7, 12), 'W'))));
    let m = decode::exif_meta(&pic);
    assert_eq!(m.taken.as_deref(), Some("2024-07-14T18:30:05+01:00"), "the original date, with its zone");
    assert_eq!((m.make.as_deref(), m.model.as_deref()), (Some("Canon"), Some("EOS R6 2")));
    assert!((m.lat.unwrap() - 51.51).abs() < 1e-9 && (m.lon.unwrap() + 0.12).abs() < 1e-9, "{:?} {:?}", m.lat, m.lon);
    assert_eq!(decode::exif_meta(&jpeg(&halves(8, 8))), Meta { orientation: 1, ..Meta::default() }, "no EXIF: defaults");
}

#[test]
fn decompression_bombs_are_refused_before_decoding() {
    // A real PNG of 30000 × 30000 one-bit pixels: about 110 KB on disk, 900 million pixels.
    let mut png = Vec::new();
    {
        let mut e = png::Encoder::new(&mut png, 30_000, 30_000);
        e.set_color(png::ColorType::Grayscale);
        e.set_depth(png::BitDepth::One);
        e.set_compression(png::Compression::Fast);
        let mut w = e.write_header().unwrap();
        let row = vec![0u8; 30_000 / 8];
        let mut s = w.stream_writer().unwrap();
        for _ in 0..30_000 {
            std::io::Write::write_all(&mut s, &row).unwrap();
        }
        s.finish().unwrap();
    }
    assert!(png.len() < 2_000_000, "{} bytes", png.len());
    let t = Instant::now();
    let e = decode::make_guarded(&png, &LIMITS, None).err().unwrap();
    assert!(e.contains("too big"), "{e}");
    assert!(t.elapsed() < Duration::from_millis(500), "refused from the header, not decoded: {:?}", t.elapsed());
    // A long thin one (under the pixel limit, over the side limit) and a JPEG claiming 60000².
    let thin = encoded(&image::DynamicImage::ImageLuma8(image::GrayImage::new(40_000, 1)), image::ImageFormat::Png);
    assert!(decode::make(&thin, &LIMITS).unwrap_err().contains("too big"));
    let mut big = jpeg(&halves(16, 16));
    let sof = big.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
    big[sof + 5..sof + 9].copy_from_slice(&[0xEA, 0x60, 0xEA, 0x60]);
    assert!(decode::make_guarded(&big, &LIMITS, None).unwrap_err().contains("too big"));
    // Tighter limits are honoured.
    let tight = decode::Limits { max_pixels: 100, max_pixels_jpeg: 100, ..LIMITS };
    assert!(decode::make(&jpeg(&halves(11, 10)), &tight).unwrap_err().contains("too big"));
}

#[test]
fn a_child_that_hangs_or_crashes_is_one_failed_picture() {
    let t = Instant::now();
    let mut sleep = std::process::Command::new("sleep");
    sleep.arg("30");
    let e = decode::run_child(sleep, decode::Input::Bytes(b"x".to_vec()), Duration::from_millis(300)).err().unwrap();
    assert!(e.contains("stopped"), "{e}");
    assert!(t.elapsed() < Duration::from_secs(5), "killed, not waited for");
    let mut crash = std::process::Command::new("sh");
    crash.args(["-c", "ulimit -c 0; kill -SEGV $$"]);
    let e = decode::run_child(crash, decode::Input::Bytes(b"x".to_vec()), Duration::from_secs(5)).err().unwrap();
    assert!(e.contains("signal"), "{e}");
    let mut nonsense = std::process::Command::new("echo");
    nonsense.arg("{not json");
    assert!(decode::run_child(nonsense, decode::Input::Bytes(Vec::new()), Duration::from_secs(5)).is_err());
}

fn config(workers: usize, watch: bool) -> Config {
    Config { workers, child_memory: decode::CHILD_MEMORY, rescan: Some(86_400), cache: None, cache_cap: 1 << 30, watch }
}

/// A scan of everything Photos is on for, through the scanner thread, waited for.
fn rescan(svc: &Photos) {
    let t = svc.request_scan("", true, true);
    assert!(svc.wait_scan(t, Duration::from_secs(30)), "scan didn't finish");
}

/// A store with pictures, Photos on for ann/Trip and bob: (store, service).
fn with_pictures() -> (Store, Arc<Photos>) {
    let root = tmp("store");
    let s = Store { root: root.clone(), keep: 3, spaces: true };
    let put = |p: &str, b: &[u8]| {
        let f = s.full(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, b).unwrap();
    };
    put("ann/Trip/beach.jpg", &with_exif(&jpeg(&halves(1600, 1200)), &exif_block(1, "2024:07:14 18:30:05", None)));
    put("ann/Trip/sunset.JPG", &with_exif(&jpeg(&halves(800, 600)), &exif_block(8, "2023:01:02 03:04:05", None)));
    put("ann/Trip/map.png", &encoded(&image::DynamicImage::ImageRgb8(halves(300, 200)), image::ImageFormat::Png));
    put("ann/Trip/iphone.heic", b"not decoded yet");
    put("ann/Trip/broken.jpg", b"\xFF\xD8\xFF\xE0 this is not really a jpeg");
    put("ann/Trip/notes.txt", b"not a picture");
    put("ann/Trip/.hidden.jpg", &jpeg(&halves(8, 8)));
    put("ann/Private/me.jpg", &jpeg(&halves(8, 8)));
    put("ann/.bin/old.jpg", &jpeg(&halves(8, 8)));
    put("bob/me.gif", &encoded(&image::DynamicImage::ImageRgb8(halves(30, 30)), image::ImageFormat::Gif));
    std::os::unix::fs::symlink(s.full("bob/me.gif"), s.full("ann/Trip/link.gif")).unwrap();
    std::os::unix::fs::symlink(s.full("ann/Private"), s.full("ann/Trip/linked-folder")).unwrap();
    let svc = Photos::start(&s, config(2, false)).unwrap();
    (s, svc)
}

fn settle(svc: &Photos) {
    let t = Instant::now();
    while svc.queued() != (0, 0) || svc.db().counts().unwrap()[0] > 0 {
        assert!(t.elapsed() < Duration::from_secs(60), "still working: {:?}", svc.status());
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn nothing_is_indexed_until_photos_is_turned_on() {
    let (s, svc) = with_pictures();
    rescan(&svc);
    assert!(svc.rows("", true).is_empty(), "off by default: nothing read");
    assert!(!svc.covers("ann/Trip") && !svc.hurry("ann/Trip/beach.jpg"));
    svc.enable("ann/Trip", "ann").unwrap();
    rescan(&svc);
    settle(&svc);
    assert!(svc.covers("ann/Trip/Day 2/x.jpg"), "a folder inside is covered too");
    assert!(svc.rows("ann/Private", true).is_empty(), "other folders stay out");
    assert_eq!(svc.status()["folders"], json!(["ann/Trip"]));
    svc.stop();
    let _ = std::fs::remove_dir_all(&s.root);
}

#[test]
fn the_index_and_thumbnails_of_a_store() {
    let (s, svc) = with_pictures();
    svc.enable("ann/Trip", "ann").unwrap();
    svc.enable("bob", "bob").unwrap();
    rescan(&svc);
    settle(&svc);
    let rows: HashMap<String, index::Row> = svc.rows("", true).into_iter().map(|r| (r.path.clone(), r)).collect();
    let mut paths: Vec<&str> = rows.keys().map(String::as_str).collect();
    paths.sort();
    assert_eq!(
        paths,
        ["ann/Trip/beach.jpg", "ann/Trip/broken.jpg", "ann/Trip/iphone.heic", "ann/Trip/map.png", "ann/Trip/sunset.JPG", "bob/me.gif"],
        "no hidden files, bins, text, anything through a link, or folders Photos is off for"
    );
    let beach = &rows["ann/Trip/beach.jpg"];
    assert_eq!((beach.state, beach.meta.width, beach.meta.taken.as_deref()), (State::Ready, 1600, Some("2024-07-14T18:30:05+01:00")));
    assert_eq!(rows["ann/Trip/sunset.JPG"].meta.orientation, 8);
    assert_eq!((rows["ann/Trip/sunset.JPG"].meta.width, rows["ann/Trip/sunset.JPG"].meta.height), (600, 800));
    assert_eq!(rows["ann/Trip/iphone.heic"].state, State::Unsupported);
    assert_eq!(rows["ann/Trip/broken.jpg"].state, State::Failed);
    assert!(rows["ann/Trip/broken.jpg"].error.is_some());
    for p in ["ann/Trip/beach.jpg", "ann/Trip/map.png", "bob/me.gif"] {
        let hash = rows[p].hash.as_deref().unwrap();
        for size in decode::SIZES {
            let (file, _) = svc.thumb_file(hash, size).unwrap_or_else(|| panic!("{p} {size}"));
            assert!(file.starts_with(s.meta()), "kept beside the server's state, never in people's folders");
            let img = image::open(&file).unwrap();
            assert!(img.width().max(img.height()) <= size);
        }
    }
    assert!(
        !s.full("ann/Trip").read_dir().unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains("256")),
        "nothing written into the folder"
    );
    // A rename keeps the row (and its id); a copy reuses the thumbnails; a change is read again.
    let id = beach.id;
    std::fs::rename(s.full("ann/Trip/beach.jpg"), s.full("ann/Trip/beach 2024.jpg")).unwrap();
    std::fs::copy(s.full("ann/Trip/map.png"), s.full("ann/Trip/map copy.png")).unwrap();
    let made_before = svc.made.load(Ordering::Relaxed);
    rescan(&svc);
    settle(&svc);
    let moved = svc.row("ann/Trip/beach 2024.jpg").unwrap();
    assert_eq!((moved.id, moved.state), (id, State::Ready), "renamed: same row");
    assert_eq!(svc.row("ann/Trip/map copy.png").unwrap().state, State::Ready);
    assert_eq!(svc.made.load(Ordering::Relaxed), made_before, "nothing decoded again");
    std::fs::write(s.full("ann/Trip/map.png"), encoded(&image::DynamicImage::ImageRgb8(halves(50, 40)), image::ImageFormat::Png)).unwrap();
    let t = svc.look_at("ann/Trip", false).unwrap();
    assert!(svc.wait_scan(t, Duration::from_secs(10)));
    settle(&svc);
    assert_eq!(svc.row("ann/Trip/map.png").unwrap().meta.width, 50, "changed: read again");
    // A deleted picture leaves the index, and its thumbnails go at once.
    let gif = rows["bob/me.gif"].hash.clone().unwrap();
    std::fs::remove_file(s.full("bob/me.gif")).unwrap();
    rescan(&svc);
    assert!(svc.row("bob/me.gif").is_none(), "deleted: gone from the index");
    assert!(svc.thumb_file(&gif, 256).is_none(), "and its thumbnails");
    // Turned off: the folder's pictures and their thumbnails are forgotten.
    let sunset = rows["ann/Trip/sunset.JPG"].hash.clone().unwrap();
    assert!(!svc.disable("ann/Trip").unwrap());
    assert!(svc.rows("ann", true).is_empty() && svc.thumb_file(&sunset, 1024).is_none());
    assert_eq!(svc.status()["pictures"]["unsupported"], 0);
    svc.stop();
    let _ = std::fs::remove_dir_all(&s.root);
}

#[test]
fn the_cache_keeps_to_its_size() {
    let (s, svc) = with_pictures();
    svc.enable("ann/Trip", "ann").unwrap();
    rescan(&svc);
    settle(&svc);
    svc.clear_unused();
    let full = svc.cache_bytes.load(Ordering::Relaxed);
    assert!(full > 0, "the cache's size is counted");
    // A cap below what is there: the least recently shown go, down to 90% of the cap.
    let small = Photos { cfg: Config { cache_cap: full / 2, ..svc.cfg.clone() }, ..clone_for_test(&svc) };
    small.clear_unused();
    assert!(small.cache_bytes.load(Ordering::Relaxed) <= full / 2, "{} of {full}", small.cache_bytes.load(Ordering::Relaxed));
    svc.stop();
    let _ = std::fs::remove_dir_all(&s.root);
}

/// A second service over the same store and cache, without threads (for a different setting).
fn clone_for_test(p: &Photos) -> Photos {
    Photos {
        store: Store { root: p.store.root.clone(), keep: 3, spaces: p.store.spaces },
        cache: p.cache.clone(),
        db: Mutex::new(Index::open(&tmp("other-index").join("i.sqlite")).unwrap()),
        queue: Mutex::default(),
        wake: Condvar::new(),
        cfg: p.cfg.clone(),
        network: false,
        enabled: Mutex::default(),
        scans: Mutex::default(),
        scans_wake: Condvar::new(),
        watching: Mutex::new(String::new()),
        watcher: Mutex::new(None),
        dirty: Mutex::default(),
        looked: Mutex::default(),
        last_scan: Mutex::new(None),
        scanning: AtomicBool::new(false),
        made: AtomicU64::new(0),
        cache_bytes: AtomicU64::new(0),
        stop: AtomicBool::new(true),
    }
}

#[test]
fn workers_and_memory_fit_the_server() {
    let gb = 1024 * 1024 * 1024;
    assert_eq!(fit_memory(2, decode::CHILD_MEMORY, None), (2, decode::CHILD_MEMORY), "no limit: as asked");
    assert_eq!(fit_memory(2, decode::CHILD_MEMORY, Some(8 * gb)), (2, decode::CHILD_MEMORY), "plenty");
    assert_eq!(fit_memory(2, decode::CHILD_MEMORY, Some(2 * gb)), (2, 512 * 1024 * 1024), "less memory per child first");
    assert_eq!(fit_memory(4, decode::CHILD_MEMORY, Some(gb + gb / 2)), (1, decode::MIN_CHILD_MEMORY), "then fewer workers");
    assert_eq!(fit_memory(2, decode::CHILD_MEMORY, Some(gb / 4)), (1, decode::MIN_CHILD_MEMORY), "never below one");
}

#[test]
fn inotify_sees_a_new_picture() {
    let root = tmp("watch");
    let s = Store { root: root.clone(), keep: 3, spaces: false };
    std::fs::create_dir_all(s.full("Camera")).unwrap();
    std::fs::create_dir_all(s.full("Elsewhere")).unwrap();
    let svc = Photos::start(&s, config(1, true)).unwrap();
    svc.enable("Camera", "").unwrap();
    rescan(&svc);
    assert_eq!(svc.status()["watching"], "inotify");
    std::fs::write(s.full("Camera/new.jpg"), jpeg(&halves(20, 10))).unwrap();
    std::fs::write(s.full("Elsewhere/other.jpg"), jpeg(&halves(20, 10))).unwrap();
    let t = Instant::now();
    while svc.row("Camera/new.jpg").is_none_or(|r| r.state != State::Ready) {
        assert!(t.elapsed() < Duration::from_secs(15), "not seen");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(svc.row("Elsewhere/other.jpg").is_none(), "only folders Photos is on for");
    svc.stop();
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------- through the router

#[path = "edits_tests.rs"]
mod edits_tests;
#[path = "export_tests.rs"]
mod export_tests;

mod http {
    use super::*;
    use crate::{Shared, auth, sharing};
    use axum::body::Body;
    use axum::http::{HeaderMap, header};
    use tower::ServiceExt;

    pub(super) struct Got {
        pub status: u16,
        pub headers: HeaderMap,
        pub json: Value,
        pub bytes: Vec<u8>,
    }

    pub(super) async fn ask(app: &Shared, method: &str, uri: &str, who: Option<&str>, extra: &[(header::HeaderName, &str)]) -> Got {
        send(app, method, uri, who, extra, "").await
    }

    pub(super) async fn send(app: &Shared, method: &str, uri: &str, who: Option<&str>, extra: &[(header::HeaderName, &str)], body: &str) -> Got {
        let mut b = axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header(header::HOST, "192.168.1.5:8080")
            .header(header::ORIGIN, "http://192.168.1.5:8080");
        if let Some(u) = who {
            let token = auth::new_session(&app.store, u).unwrap();
            b = b.header(header::COOKIE, format!("{}={token}", crate::COOKIE));
        }
        for (k, v) in extra {
            b = b.header(k, *v);
        }
        let mut req = b.body(Body::from(body.to_string())).unwrap();
        req.extensions_mut().insert(axum::extract::ConnectInfo(std::net::SocketAddr::new([10, 9, 8, 6].into(), 50000)));
        let resp = crate::router(app.clone()).oneshot(req).await.unwrap();
        let status = resp.status().as_u16();
        let headers = resp.headers().clone();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap().to_vec();
        Got { status, headers, json: serde_json::from_slice(&bytes).unwrap_or(Value::Null), bytes }
    }

    pub(super) async fn get(app: &Shared, uri: &str, who: Option<&str>, extra: &[(header::HeaderName, &str)]) -> Got {
        ask(app, "GET", uri, who, extra).await
    }

    #[tokio::test]
    async fn only_people_who_may_see_a_picture_get_it() {
        let root = tmp("http");
        let s = Store { root: root.clone(), keep: 3, spaces: true };
        for (u, n) in [("boss", "Boss"), ("ann", "Ann"), ("bob", "Bob")] {
            auth::add(&s, u, n, &format!("{u}password")).unwrap();
        }
        let put = |p: &str, b: Vec<u8>| {
            std::fs::create_dir_all(s.full(p).parent().unwrap()).unwrap();
            std::fs::write(s.full(p), b).unwrap();
        };
        put("ann/Trip/b.jpg", with_exif(&jpeg(&halves(600, 400)), &exif_block(1, "2024:07:14 18:30:05", None)));
        put("ann/Trip/a.jpg", with_exif(&jpeg(&halves(600, 400)), &exif_block(1, "2021:01:01 00:00:00", Some(((51, 30, 0), 'N', (0, 7, 0), 'W')))));
        put("ann/Trip/Day 2/c.png", encoded(&image::DynamicImage::ImageRgb8(halves(30, 20)), image::ImageFormat::Png));
        put("ann/Trip/plan.xlsx", b"x".to_vec());
        put("ann/Private/secret.jpg", jpeg(&halves(30, 20)));
        let ids: HashMap<String, String> = s.synced_index().unwrap().into_iter().map(|(id, p)| (p, id)).collect();
        let app: Shared = Arc::new(crate::App {
            store: s,
            base: root.clone(),
            lock: std::sync::Mutex::new(()),
            location: "here".into(),
            auth: true,
            site: None,
            rooms: crate::live::Rooms::default(),
            oidc: None,
        });
        let a = &ids["ann/Trip/a.jpg"];

        // Off until turned on: listed as off, nothing read, no thumbnails.
        let g = get(&app, "/api/photos?folder=ann/Trip", Some("ann"), &[]).await;
        assert_eq!((g.status, g.json["enabled"].as_bool(), g.json["can_enable"].as_bool()), (200, Some(false), Some(true)), "{}", g.json);
        assert_eq!(g.json["photos"], json!([]));
        let g = get(&app, &format!("/api/photos/thumb/{a}/256"), Some("ann"), &[]).await;
        assert_eq!((g.status, g.json["state"].as_str()), (404, Some("off")));
        assert_eq!(get(&app, "/api/photos/enable?folder=ann/Trip", Some("ann"), &[]).await.status, 405, "a GET changes nothing");
        assert_eq!(ask(&app, "POST", "/api/photos/enable?folder=ann/Trip", Some("bob"), &[]).await.status, 403, "not Bob's");
        let g = ask(&app, "POST", "/api/photos/enable?folder=ann/Trip", Some("ann"), &[]).await;
        assert_eq!((g.status, g.json["enabled"].as_bool()), (200, Some(true)), "{}", g.json);

        let g = get(&app, "/api/photos?folder=ann/Trip", Some("ann"), &[]).await;
        assert_eq!(g.status, 200, "{}", g.json);
        assert_eq!(g.json["photos"].as_array().unwrap().len(), 2, "listed at once, read or not: {}", g.json);
        // Asked again once both are read (dates come from EXIF).
        let t = Instant::now();
        let g = loop {
            let g = get(&app, "/api/photos?folder=ann/Trip", Some("ann"), &[]).await;
            if g.json["photos"].as_array().unwrap().iter().all(|p| p["state"] == "ready") || t.elapsed() > Duration::from_secs(10) {
                break g;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        let names: Vec<&str> = g.json["photos"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["a.jpg", "b.jpg"], "by date taken; no spreadsheets; not the folder inside");
        assert_eq!(g.json["photos"][0]["taken"], "2021-01-01T00:00:00+01:00");
        assert!(g.json["photos"][0]["gps"]["lat"].as_f64().is_some(), "the owner sees where: {}", g.json);
        // With the folders inside (rescanned in the background: asked again until it's there).
        let t = Instant::now();
        loop {
            let g = get(&app, "/api/photos?folder=ann/Trip&sub=1&sort=name&desc=1", Some("ann"), &[]).await;
            let names: Vec<String> = g.json["photos"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap().to_string()).collect();
            if names.len() == 3 || t.elapsed() > Duration::from_secs(10) {
                assert_eq!(names, ["c.png", "b.jpg", "a.jpg"]);
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(get(&app, "/api/photos?folder=ann/Trip", None, &[]).await.status, 401, "signed in only");
        assert_eq!(get(&app, "/api/photos?folder=ann/Trip", Some("bob"), &[]).await.status, 403);
        assert_eq!(get(&app, "/api/photos?folder=ann", Some("bob"), &[]).await.status, 403);
        assert_eq!(get(&app, "/api/photos?folder=ann/.bin", Some("ann"), &[]).await.status, 400);
        assert_eq!(get(&app, "/api/photos?folder=ann/Nowhere", Some("ann"), &[]).await.status, 404);

        // The thumbnail: waited for (a few seconds at most), then kept by the browser by hash.
        let g = get(&app, &format!("/api/photos/thumb/{a}/256"), Some("ann"), &[]).await;
        assert_eq!(g.status, 200, "{}", g.json);
        assert_eq!(g.headers[header::CONTENT_TYPE], "image/jpeg");
        assert_eq!(g.headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(g.headers[header::CACHE_CONTROL], "private, no-cache", "no v: checked each time");
        assert!(g.headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap().starts_with("sandbox"));
        assert_eq!(image::load_from_memory(&g.bytes).unwrap().width(), 256);
        let listed = get(&app, "/api/photos?folder=ann/Trip", Some("ann"), &[]).await;
        let v = listed.json["photos"][0]["v"].as_str().unwrap().to_string();
        let g = get(&app, &format!("/api/photos/thumb/{a}/1024?v={v}"), Some("ann"), &[]).await;
        assert_eq!(g.headers[header::CACHE_CONTROL], "private, max-age=31536000, immutable");
        assert_eq!(image::load_from_memory(&g.bytes).unwrap().width(), 600, "never enlarged");
        let etag = g.headers[header::ETAG].to_str().unwrap().to_string();
        assert_eq!(get(&app, &format!("/api/photos/thumb/{a}/1024"), Some("ann"), &[(header::IF_NONE_MATCH, &etag)]).await.status, 304);
        assert_eq!(get(&app, &format!("/api/photos/thumb/{a}/512"), Some("ann"), &[]).await.status, 400, "only the two sizes");
        assert_eq!(get(&app, &format!("/api/photos/thumb/{a}/256"), Some("bob"), &[]).await.status, 403);
        assert_eq!(get(&app, "/api/photos/thumb/nosuchid/256", Some("ann"), &[]).await.status, 403);
        let x = &ids["ann/Trip/plan.xlsx"];
        assert_eq!(get(&app, &format!("/api/photos/thumb/{x}/256"), Some("ann"), &[]).await.status, 404, "not a picture");

        // Shared with Bob (view): the folder and its pictures (not where they were taken), nothing
        // else of Ann's; he can't turn Photos on or off.
        let folder_id = app.store.synced_folders().unwrap().into_iter().find(|(_, p)| p == "ann/Trip").unwrap().0;
        sharing::set(
            &app.store,
            sharing::Item::Folder(&folder_id),
            &sharing::Share { people: vec![("bob".into(), sharing::Role::View)], link: sharing::Role::None },
        )
        .unwrap();
        let g = get(&app, &format!("/api/photos?folder_id={folder_id}"), Some("bob"), &[]).await;
        assert_eq!((g.status, g.json["photos"].as_array().map(Vec::len)), (200, Some(2)), "{}", g.json);
        assert!(g.json["photos"].as_array().unwrap().iter().all(|p| p["gps"].is_null()), "no places for viewers: {}", g.json);
        assert_eq!(g.json["can_enable"], false);
        assert_eq!(get(&app, &format!("/api/photos/thumb/{a}/256"), Some("bob"), &[]).await.status, 200);
        let secret = &ids["ann/Private/secret.jpg"];
        assert_eq!(get(&app, &format!("/api/photos/thumb/{secret}/256"), Some("bob"), &[]).await.status, 403);
        assert_eq!(get(&app, "/api/photos?folder=ann/Private", Some("bob"), &[]).await.status, 403);
        assert_eq!(ask(&app, "POST", "/api/photos/disable?folder=ann/Trip", Some("bob"), &[]).await.status, 403);

        // Status: the server's owner only.
        let g = get(&app, "/api/photos/status", Some("boss"), &[]).await;
        assert_eq!(g.status, 200);
        assert!(g.json["pictures"]["ready"].as_u64().is_some() && g.json["cache"]["bytes"].as_u64().is_some(), "{}", g.json);
        assert_eq!(get(&app, "/api/photos/status", Some("bob"), &[]).await.status, 403);
        assert_eq!(get(&app, "/api/photos/status", Some("ann"), &[]).await.status, 403);

        // Ann turns it off: forgotten.
        let g = ask(&app, "POST", "/api/photos/disable?folder=ann/Trip", Some("ann"), &[]).await;
        assert_eq!((g.status, g.json["enabled"].as_bool()), (200, Some(false)), "{}", g.json);
        assert_eq!(get(&app, &format!("/api/photos/thumb/{a}/256"), Some("ann"), &[]).await.json["state"], "off");
        if let Some(svc) = service(&app.store) {
            assert!(svc.rows("ann", true).is_empty());
            svc.stop();
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[test]
fn cache_dir_choice() {
    use std::path::{Path, PathBuf};
    let root = Path::new("/data");
    let meta = Path::new("/data/.starsheets");
    let pick = |set: Option<&str>, mounted: bool, net: bool, xdg: Option<&str>, home: Option<&str>| {
        super::pick_cache(set.map(PathBuf::from), mounted, root, meta, |_| net, xdg.map(PathBuf::from), home.map(PathBuf::from)).0
    };
    assert_eq!(pick(Some("/x"), true, true, None, None), PathBuf::from("/x"));
    assert_eq!(pick(None, true, true, None, None), PathBuf::from("/cache/photos"));
    let n = pick(None, false, true, Some("/xdg"), Some("/home/u"));
    assert!(n.starts_with("/xdg/starbase/photos"), "{n:?}");
    assert!(pick(None, false, true, None, Some("/home/u")).starts_with("/home/u/.cache/starbase/photos"));
    assert!(pick(None, false, true, None, None).starts_with("/var/cache/starbase/photos"));
    assert_eq!(pick(None, false, false, Some("/xdg"), None), PathBuf::from("/data/.starsheets/photos-cache"));
}
