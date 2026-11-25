//! Edited thumbnails and "Save a copy": the edits are rendered on the upright picture, location
//! data never leaves by default (kamadak-exif reads the result back; exiftool too where it is
//! installed), and exports are new files only for people who may add them.

use super::export::{self, Format, Options};
use super::http::{ask, send};
use super::*;
use crate::{Shared, auth, sharing};
use starbase_photo_edit::Edits;

fn edits(j: &str) -> Edits {
    serde_json::from_str(j).unwrap()
}

/// A JPEG `w` × `h` (stored), EXIF orientation `o`, a camera, a date and a GPS position, plus an
/// XMP packet that also has a position (where phones and apps put a second copy).
fn photo(w: u32, h: u32, o: u16) -> Vec<u8> {
    let with = with_exif(&jpeg(&halves(w, h)), &exif_block(o, "2024:07:14 18:30:05", Some(((51, 30, 0), 'N', (0, 7, 0), 'W'))));
    let xmp = br#"http://ns.adobe.com/xap/1.0/ <x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:exif="http://ns.adobe.com/exif/1.0/" exif:GPSLatitude="51,30.0N" exif:GPSLongitude="0,7.0W"/></rdf:RDF></x:xmpmeta>"#;
    let mut seg = xmp.to_vec();
    seg[28] = 0; // the namespace name ends with a zero byte
    let mut out = with[..2].to_vec();
    out.extend([0xFF, 0xE1]);
    out.extend(((seg.len() + 2) as u16).to_be_bytes());
    out.extend(seg);
    out.extend(&with[2..]);
    out
}

fn exif_of(bytes: &[u8]) -> Vec<(exif::Tag, String)> {
    let ex = exif::Reader::new().read_from_container(&mut std::io::Cursor::new(bytes)).unwrap();
    ex.fields().map(|f| (f.tag, f.display_value().to_string())).collect()
}

#[test]
fn thumbnails_show_the_edits_on_the_upright_picture() {
    // Stored 300 × 200, shown turned a quarter (EXIF 6): 200 × 300. The crop keeps the shown top
    // half: 200 × 150, which the 256 thumbnail keeps as it is.
    let pic = with_exif(&jpeg(&halves(300, 200)), &exif_block(6, "2024:07:14 18:30:05", None));
    let e = edits(r#"{"crop": {"l": 0, "t": 0, "r": 1, "b": 0.5}}"#);
    let made = decode::make_guarded(&pic, &LIMITS, Some(&e)).unwrap();
    let t = made.thumbs.iter().find(|t| t.size == 256).unwrap();
    let img = image::load_from_memory(&t.bytes).unwrap().into_rgb8();
    assert_eq!(img.dimensions(), (200, 150));
    // Stored left half red: turned clockwise, red is on top. All of the kept top half is red.
    let p = img.get_pixel(100, 75);
    assert!(p[0] > 200 && p[2] < 60, "{p:?}");
    // No edits: the same thumbnails as before edits existed (byte for byte).
    let plain = decode::make(&pic, &LIMITS).unwrap();
    let none = decode::make_guarded(&pic, &LIMITS, Some(&Edits::default())).unwrap();
    assert_eq!(plain.thumbs.iter().map(|t| &t.bytes).collect::<Vec<_>>(), none.thumbs.iter().map(|t| &t.bytes).collect::<Vec<_>>());
    // A tight crop of a big picture is decoded bigger, so the 1024 thumbnail is still sharp.
    let big = jpeg(&halves(4000, 3000));
    let e = edits(r#"{"crop": {"l": 0.4, "t": 0.4, "r": 0.6, "b": 0.6}}"#);
    let made = decode::make_guarded(&big, &LIMITS, Some(&e)).unwrap();
    let t = made.thumbs.iter().find(|t| t.size == 1024).unwrap();
    let w = image::load_from_memory(&t.bytes).unwrap().width();
    assert!(w >= 800, "{w}");
}

#[test]
fn exports_are_upright_edited_and_keep_no_location() {
    let pic = photo(300, 200, 6);
    let e = edits(r#"{"rot90": 1, "exposure": 0.5}"#);
    let out = export::export(&pic, &LIMITS, &e, &Options::default()).unwrap();
    let img = image::load_from_memory(&out).unwrap();
    // EXIF 6 then a quarter turn of ours: upside down, 300 × 200.
    assert_eq!((img.width(), img.height()), (300, 200));
    let tags = exif_of(&out);
    let has = |t: exif::Tag| tags.iter().find(|(x, _)| *x == t).map(|(_, v)| v.clone());
    assert!(has(exif::Tag::Orientation).is_some());
    assert_eq!(decode::exif_meta(&out).orientation, 1, "the pixels are upright");
    assert!(has(exif::Tag::Make).is_some_and(|m| m.contains("Canon")), "{tags:?}");
    assert!(has(exif::Tag::DateTimeOriginal).is_some(), "{tags:?}");
    assert!(!tags.iter().any(|(t, _)| t.to_string().starts_with("GPS")), "no GPS: {tags:?}");
    assert!(!out.windows(9).any(|w| w == b"GPSLatitu") && !out.windows(5).any(|w| w == b"xmpme"), "no XMP at all");
    // Asked to keep the location: the EXIF position only.
    let keep = Options { strip_location: false, ..Options::default() };
    let out = export::export(&pic, &LIMITS, &e, &keep).unwrap();
    assert!(exif_of(&out).iter().any(|(t, _)| *t == exif::Tag::GPSLatitude));
    // Smaller, and PNG.
    let o = Options { format: Format::Png, max_side: Some(100), ..Options::default() };
    let out = export::export(&pic, &LIMITS, &e, &o).unwrap();
    assert!(out.starts_with(b"\x89PNG"));
    let img = image::load_from_memory(&out).unwrap();
    assert_eq!((img.width(), img.height()), (100, 67));
    // Bad settings and pictures.
    assert!(export::export(&pic, &LIMITS, &e, &Options { quality: 0, ..Options::default() }).is_err());
    assert!(export::export(b"not a picture", &LIMITS, &e, &Options::default()).is_err());
    let tight = decode::Limits { max_pixels: 1000, ..LIMITS };
    assert!(export::export(&pic, &tight, &e, &Options::default()).unwrap_err().contains("too big"));
}

/// exiftool (the reference reader) finds no location in an export. CI installs it; it FAILS where
/// it is missing unless STARBASE_SKIP_EXIFTOOL=1 (a laptop without it: `apt install libimage-exiftool-perl`).
#[test]
fn exiftool_finds_no_location() {
    if std::process::Command::new("exiftool").arg("-ver").output().map(|o| !o.status.success()).unwrap_or(true) {
        assert!(std::env::var("STARBASE_SKIP_EXIFTOOL").is_ok_and(|v| v == "1"), "exiftool is not installed (set STARBASE_SKIP_EXIFTOOL=1 to skip)");
        eprintln!("exiftool not installed: skipped (STARBASE_SKIP_EXIFTOOL=1)");
        return;
    }
    let dir = tmp("exiftool");
    let pic = photo(300, 200, 6);
    let tags = |name: &str, bytes: &[u8]| -> Value {
        let f = dir.join(name);
        std::fs::write(&f, bytes).unwrap();
        let o = std::process::Command::new("exiftool").args(["-j", "-a", "-G1", "-n"]).arg(&f).output().unwrap();
        serde_json::from_slice::<Value>(&o.stdout).unwrap()[0].clone()
    };
    let gps = |v: &Value| v.as_object().unwrap().keys().filter(|k| k.to_ascii_lowercase().contains("gps")).cloned().collect::<Vec<_>>();
    // The original has it twice (EXIF and XMP): the oracle sees both.
    let orig = tags("orig.jpg", &pic);
    assert!(gps(&orig).iter().any(|k| k.starts_with("XMP")) && gps(&orig).iter().any(|k| k.starts_with("GPS")), "{orig}");
    let out = export::export(&pic, &LIMITS, &Edits::default(), &Options::default()).unwrap();
    let got = tags("out.jpg", &out);
    assert!(gps(&got).is_empty(), "location left in: {got}");
    assert_eq!(got["IFD0:Orientation"], 1, "{got}");
    assert_eq!(got["IFD0:Make"], "Canon", "{got}");
    assert!(got.as_object().unwrap().keys().all(|k| !k.starts_with("XMP") && !k.starts_with("MakerNotes")), "{got}");
    let kept = tags("kept.jpg", &export::export(&pic, &LIMITS, &Edits::default(), &Options { strip_location: false, ..Options::default() }).unwrap());
    assert!(kept.get("GPS:GPSLatitude").is_some(), "{kept}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn exports_are_new_files_for_people_who_may_add_them() {
    let root = tmp("export");
    let s = Store { root: root.clone(), keep: 3, spaces: true };
    for (u, n) in [("boss", "Boss"), ("ann", "Ann"), ("bob", "Bob")] {
        auth::add(&s, u, n, &format!("{u}password")).unwrap();
    }
    std::fs::create_dir_all(s.full("ann/Trip")).unwrap();
    std::fs::write(s.full("ann/Trip/a.jpg"), photo(60, 40, 1)).unwrap();
    std::fs::write(s.full("ann/Trip/raw.cr2"), b"raw").unwrap();
    std::fs::write(s.full("ann/Trip/notes.txt"), b"text").unwrap();
    // Saved edits: a quarter turn.
    let e = edits(r#"{"rot90": 1}"#);
    let sidecar::Merged::Write(x) = sidecar::merge(None, Some(&e), &sidecar::Geo { width: 60, height: 40, orientation: 1 }).unwrap() else {
        panic!()
    };
    std::fs::write(s.full("ann/Trip/a.jpg.xmp"), x).unwrap();
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
    let url = |id: &str| format!("/api/photos/export/{id}");
    let a = &ids["ann/Trip/a.jpg"];
    let g = send(&app, "POST", &url(a), Some("ann"), &[], "").await;
    assert_eq!((g.status, g.json["path"].as_str()), (201, Some("ann/Trip/a (edited).jpg")), "{}", g.json);
    let made = std::fs::read(app.store.full("ann/Trip/a (edited).jpg")).unwrap();
    let img = image::load_from_memory(&made).unwrap();
    assert_eq!((img.width(), img.height()), (40, 60), "the saved edits are in it");
    assert!(!exif_of(&made).iter().any(|(t, _)| t.to_string().starts_with("GPS")));
    assert!(app.store.synced_index().unwrap().values().any(|p| p == "ann/Trip/a (edited).jpg"), "an ordinary file with an id");
    // Never over an existing file: numbered.
    let g = send(&app, "POST", &url(a), Some("ann"), &[], r#"{"format": "png", "max_side": 30}"#).await;
    assert_eq!((g.status, g.json["name"].as_str()), (201, Some("a (edited).png")), "{}", g.json);
    let g = send(&app, "POST", &url(a), Some("ann"), &[], "{}").await;
    assert_eq!(g.json["name"].as_str(), Some("a (edited) (2).jpg"), "{}", g.json);
    assert_eq!(std::fs::read(app.store.full("ann/Trip/a (edited).jpg")).unwrap(), made, "the first one untouched");
    // Settings checked.
    assert_eq!(send(&app, "POST", &url(a), Some("ann"), &[], r#"{"quality": 0}"#).await.status, 400);
    assert_eq!(send(&app, "POST", &url(a), Some("ann"), &[], r#"{"nonsense": 1}"#).await.status, 400);
    assert_eq!(send(&app, "POST", &url(a), Some("ann"), &[], &"x".repeat(17_000)).await.status, 413);
    assert_eq!(ask(&app, "GET", &url(a), Some("ann"), &[]).await.status, 405);
    // Who may: not a viewer of the folder, nobody who can't see it.
    let folder_id = app.store.synced_folders().unwrap().into_iter().find(|(_, p)| p == "ann/Trip").unwrap().0;
    sharing::set(
        &app.store,
        sharing::Item::Folder(&folder_id),
        &sharing::Share { people: vec![("bob".into(), sharing::Role::View)], link: sharing::Role::None, ..Default::default() },
    )
    .unwrap();
    let g = send(&app, "POST", &url(a), Some("bob"), &[], "").await;
    assert_eq!(g.status, 403, "{}", g.json);
    assert_eq!(send(&app, "POST", "/api/photos/export/nosuchid", Some("bob"), &[], "").await.status, 403);
    assert_eq!(send(&app, "POST", &url(a), None, &[], "").await.status, 401);
    assert_eq!(send(&app, "POST", &url(&ids["ann/Trip/notes.txt"]), Some("ann"), &[], "").await.status, 404);
    assert_eq!(send(&app, "POST", &url(&ids["ann/Trip/raw.cr2"]), Some("ann"), &[], "").await.status, 409);
    let _ = std::fs::remove_dir_all(&root);
}

/// A second export while one runs is turned away quickly (429), not left holding a request slot.
#[tokio::test]
async fn second_export_gets_429_quickly() {
    static SEM: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
    let first = export::take_slot(&SEM, std::time::Duration::from_millis(50)).await;
    assert!(first.is_some());
    let t = std::time::Instant::now();
    assert!(export::take_slot(&SEM, std::time::Duration::from_millis(100)).await.is_none());
    assert!(t.elapsed() < std::time::Duration::from_secs(2));
    drop(first);
    assert!(export::take_slot(&SEM, std::time::Duration::from_millis(50)).await.is_some());
}

/// A photo shared with someone, or by link, goes without its location (exiftool is the oracle);
/// the owner's own download keeps it; the file on disk is untouched; the owner can allow it.
#[tokio::test]
async fn shared_downloads_leave_the_location_out() {
    let root = tmp("shared-dl");
    let s = Store { root: root.clone(), keep: 3, spaces: true };
    for (u, n) in [("boss", "Boss"), ("ann", "Ann"), ("bob", "Bob")] {
        auth::add(&s, u, n, &format!("{u}password")).unwrap();
    }
    std::fs::create_dir_all(s.full("ann/Trip")).unwrap();
    let pic = photo(60, 40, 6);
    std::fs::write(s.full("ann/Trip/a.jpg"), &pic).unwrap();
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let chunk = |k: &[u8], d: &[u8]| [(d.len() as u32).to_be_bytes().to_vec(), k.to_vec(), d.to_vec(), vec![0; 4]].concat();
    png.extend(chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]));
    png.extend(chunk(b"eXIf", b"MM\0*GPSsecret"));
    png.extend(chunk(b"IEND", b""));
    std::fs::write(s.full("ann/Trip/b.png"), &png).unwrap();
    s.synced_index().unwrap();
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
    let folder = app.store.synced_folders().unwrap().into_iter().find(|(_, p)| p == "ann/Trip").unwrap().0;
    let share = |keep: bool| {
        sharing::set(&app.store, sharing::Item::Folder(&folder), &sharing::Share { people: vec![], link: sharing::Role::View, keep_location: keep })
            .unwrap()
    };
    let (a, b) = (&ids["ann/Trip/a.jpg"], &ids["ann/Trip/b.png"]);
    let gps = |bytes: &[u8]| {
        let dir = root.join("x");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("t.jpg");
        std::fs::write(&f, bytes).unwrap();
        let o =
            std::process::Command::new("exiftool").args(["-j", "-a", "-G1", "-n"]).arg(&f).output().expect("exiftool is installed (CI installs it)");
        let v: Value = serde_json::from_slice(&o.stdout).unwrap();
        v[0].as_object().unwrap().keys().filter(|k| k.to_ascii_lowercase().contains("gps")).cloned().collect::<Vec<_>>()
    };
    share(false);
    let own = ask(&app, "GET", &format!("/api/files/{a}"), Some("ann"), &[]).await;
    assert_eq!(own.bytes, pic, "the owner's download is the original");
    assert!(!gps(&own.bytes).is_empty());
    for url in [format!("/api/files/{a}"), format!("/api/files/{a}/raw")] {
        let got = ask(&app, "GET", &url, Some("bob"), &[]).await;
        assert_eq!(got.status, 200, "{url}");
        assert!(gps(&got.bytes).is_empty(), "{url}: location left in: {:?}", gps(&got.bytes));
        assert_eq!(decode::exif_meta(&got.bytes).orientation, 6, "orientation as stored");
        assert_eq!(decode::exif_meta(&got.bytes).make.as_deref(), Some("Canon"));
        assert!(image::load_from_memory(&got.bytes).is_ok());
    }
    let p = ask(&app, "GET", &format!("/api/files/{b}"), Some("bob"), &[]).await;
    assert!(
        p.status == 200 && p.bytes.starts_with(b"\x89PNG") && !p.bytes.windows(3).any(|w| w == b"GPS") && p.bytes.windows(4).any(|w| w == b"IEND"),
        "{}",
        p.status
    );
    // The owner may let it through for this share.
    share(true);
    assert!(!gps(&ask(&app, "GET", &format!("/api/files/{a}"), Some("bob"), &[]).await.bytes).is_empty());
    assert_eq!(std::fs::read(app.store.full("ann/Trip/a.jpg")).unwrap(), pic, "the source file is unchanged");
    let _ = std::fs::remove_dir_all(&root);
}
