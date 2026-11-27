//! The viewer's screen copy (2560 px): size, orientation, the scaled JPEG decode, its formats
//! and the service making it on request.

use std::time::{Duration, Instant};

use super::decode::{self, LIMITS, SCREEN, ScreenForm};
use super::tests::{encoded, exif_block, halves, jpeg, rescan, settle, with_exif, with_pictures};
use super::*;

fn dims(bytes: &[u8]) -> (u32, u32) {
    image::load_from_memory(bytes).unwrap().into_rgb8().dimensions()
}

#[test]
fn the_screen_copy_is_2560_and_never_enlarged() {
    let (out, ext, _) = decode::make_screen(&jpeg(&halves(3000, 2000)), &LIMITS, None, ScreenForm::Jpeg).unwrap();
    assert_eq!((ext, dims(&out)), ("jpg", (2560, 1707)));
    let (out, _, _) = decode::make_screen(&jpeg(&halves(40, 20)), &LIMITS, None, ScreenForm::Jpeg).unwrap();
    assert_eq!(dims(&out), (40, 20), "a small picture stays its size");
}

#[test]
fn the_screen_copy_is_upright() {
    // Stored 40 × 20 (red left, blue right); EXIF 6 = turn a quarter clockwise to show.
    let pic = with_exif(&jpeg(&halves(40, 20)), &exif_block(6, "2024:07:14 18:30:05", None));
    let (out, _, _) = decode::make_screen(&pic, &LIMITS, None, ScreenForm::Jpeg).unwrap();
    let t = image::load_from_memory(&out).unwrap().into_rgb8();
    assert_eq!(t.dimensions(), (20, 40));
    assert!(t.get_pixel(10, 3)[0] > 200 && t.get_pixel(10, 36)[2] > 200, "the left side is now at the top");
}

#[test]
fn a_50_megapixel_jpeg_is_decoded_scaled_down() {
    let big = jpeg(&halves(8192, 6144));
    let t = Instant::now();
    let (out, ext, decoded) = decode::make_screen(&big, &LIMITS, None, ScreenForm::Jpeg).unwrap();
    eprintln!("8192x6144 JPEG ({} bytes) -> 2560 copy: {:?}, {} bytes", big.len(), t.elapsed(), out.len());
    assert_eq!(decoded, (4096, 3072), "decoded at 1/2 (the smallest scale still at least 2560), not in full");
    assert_eq!((ext, dims(&out)), ("jpg", (2560, 1920)));
    let (raw, ext, _) = decode::make_screen(&big, &LIMITS, None, ScreenForm::Raw).unwrap();
    assert_eq!(ext, "ppm");
    let head = b"P6\n2560 1920\n255\n";
    assert!(raw.starts_with(head) && raw.len() == head.len() + 2560 * 1920 * 3, "a plain PPM for cjxl");
}

#[test]
fn a_transparent_picture_keeps_its_transparency() {
    let rgba = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(64, 32, image::Rgba([255, 0, 0, 100])));
    for form in [ScreenForm::Raw, ScreenForm::Jpeg] {
        let (out, ext, _) = decode::make_screen(&encoded(&rgba, image::ImageFormat::Png), &LIMITS, None, form).unwrap();
        assert_eq!(ext, "png", "{form:?}");
        assert_eq!(image::load_from_memory(&out).unwrap().into_rgba8().get_pixel(5, 5)[3], 100);
    }
}

#[test]
fn formats_best_first_and_jpeg_for_everyone_else() {
    assert_eq!(jxl::FORMATS.last(), Some(&jxl::Format::Jpeg), "the last format every browser shows");
    assert_eq!(jxl::best(false), jxl::Format::Jpeg, "a browser that didn't say it shows the best format gets JPEG");
    assert_eq!(jxl::best(true), if jxl::available() { jxl::Format::Jxl } else { jxl::Format::Jpeg });
}

#[test]
fn the_service_makes_screen_copies_on_request() {
    let (_s, svc) = with_pictures();
    rescan(&svc);
    settle(&svc);
    let key = |p: &str| svc.row(p).unwrap().thumb_key().unwrap().to_string();
    let beach = key("ann/Trip/beach.jpg");
    assert!(svc.screen_file(&beach, jxl::Format::Jpeg).is_none(), "the JPEG one only when asked for");
    svc.want_screen("ann/Trip/beach.jpg", jxl::Format::Jpeg);
    let t = Instant::now();
    let file = loop {
        if let Some((f, ext)) = svc.screen_file(&beach, jxl::Format::Jpeg) {
            assert_eq!(ext, "jpg");
            break f;
        }
        assert!(t.elapsed() < Duration::from_secs(30), "no screen copy made");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(dims(&std::fs::read(&file).unwrap()), (1600, 1200), "smaller than 2560: its own size");
    assert!(file.starts_with(svc.cache_dir().join("screen")), "its own folder, own cap");
    let original = svc.store.full("ann/Trip/beach.jpg");
    assert_eq!(dims(&std::fs::read(original).unwrap()), (1600, 1200), "the original is unchanged");
    if jxl::available() {
        // Made in the background after the thumbnails.
        let t = Instant::now();
        while svc.screen_file(&beach, jxl::Format::Jxl).is_none() {
            assert!(t.elapsed() < Duration::from_secs(60), "no JPEG XL screen copy");
            std::thread::sleep(Duration::from_millis(50));
        }
        let (f, _) = svc.screen_file(&beach, jxl::Format::Jxl).unwrap();
        assert!(std::fs::read(f).unwrap().starts_with(&[0xFF, 0x0A]), "a JPEG XL codestream");
    }
}
