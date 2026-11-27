//! The Info panel's details: EXIF read on demand, the typed fields in the sidecar, who may save.

use super::http::{Got, get, send};
use super::*;
use crate::{Shared, auth, sharing};
use axum::http::header;

/// A little-endian TIFF block: IFD0 (camera, rights, description) and the EXIF sub-IFD (exposure,
/// lens, unique id), every tag the Info panel reads.
fn rich_exif() -> Vec<u8> {
    type Entry = (u16, u16, u32, Vec<u8>);
    let ascii = |s: &str| (2u16, s.len() as u32 + 1, [s.as_bytes(), &[0]].concat());
    let rat = |n: u32, d: u32| (5u16, 1u32, [n.to_le_bytes(), d.to_le_bytes()].concat());
    let short = |n: u16| (3u16, 1u32, n.to_le_bytes().to_vec());
    let e = |tag: u16, (ty, count, v): (u16, u32, Vec<u8>)| -> Entry { (tag, ty, count, v) };
    let mut ifd0 = vec![
        e(0x010E, ascii("Sunset over the bay")),
        e(0x010F, ascii("Google")),
        e(0x0110, ascii("Pixel 8")),
        e(0x013B, ascii("Ann Example")),
        e(0x8298, ascii("(c) Ann Example")),
    ];
    let exif = vec![
        e(0x829A, rat(1, 12)),
        e(0x829D, rat(18, 10)),
        e(0x8827, short(640)),
        e(0x9003, ascii("2024:07:14 19:05:33")),
        e(0x9011, ascii("+01:00")),
        e(0x920A, rat(690, 100)),
        e(0xA405, short(26)),
        e(0xA420, ascii("ABC123UNIQUE")),
        e(0xA434, ascii("Pixel 8 back camera 6.9mm f/1.68")),
    ];
    let ifd_len = |n: usize| 2 + 12 * n as u32 + 4;
    let ifd0_at = 8u32;
    let exif_at = ifd0_at + ifd_len(ifd0.len() + 1);
    let data_at = exif_at + ifd_len(exif.len());
    ifd0.push((0x8769, 4, 1, exif_at.to_le_bytes().to_vec()));
    let mut data = Vec::new();
    let mut out = b"II\x2a\x00\x08\x00\x00\x00".to_vec();
    for entries in [&ifd0, &exif] {
        out.extend((entries.len() as u16).to_le_bytes());
        for (tag, ty, count, v) in entries {
            out.extend(tag.to_le_bytes());
            out.extend(ty.to_le_bytes());
            out.extend(count.to_le_bytes());
            if v.len() <= 4 {
                let mut v = v.clone();
                v.resize(4, 0);
                out.extend(v);
            } else {
                out.extend((data_at + data.len() as u32).to_le_bytes());
                data.extend(v);
                if data.len() % 2 == 1 {
                    data.push(0);
                }
            }
        }
        out.extend(0u32.to_le_bytes());
    }
    assert_eq!(out.len() as u32, data_at);
    out.extend(data);
    out
}

#[test]
fn exif_details_read_every_field_the_info_panel_shows() {
    let d = decode::exif_details(&with_exif(&jpeg(&halves(40, 30)), &rich_exif()));
    assert_eq!((d.make.as_deref(), d.model.as_deref()), (Some("Google"), Some("Pixel 8")));
    assert_eq!(d.lens.as_deref(), Some("Pixel 8 back camera 6.9mm f/1.68"));
    assert_eq!((d.iso, d.focal35), (Some(640), Some(26)));
    assert!((d.exposure.unwrap() - 1.0 / 12.0).abs() < 1e-9 && (d.aperture.unwrap() - 1.8).abs() < 1e-9 && (d.focal.unwrap() - 6.9).abs() < 1e-9);
    assert_eq!((d.taken.as_deref(), d.zone.as_deref()), (Some("2024-07-14T19:05:33+01:00"), Some("+01:00")));
    assert_eq!(d.unique_id.as_deref(), Some("ABC123UNIQUE"));
    assert_eq!((d.artist.as_deref(), d.copyright.as_deref()), (Some("Ann Example"), Some("(c) Ann Example")));
    assert_eq!(d.description.as_deref(), Some("Sunset over the bay"));
    // No EXIF at all: nothing, no panic.
    assert_eq!(decode::exif_details(&jpeg(&halves(10, 10))), decode::Details::default());
    assert_eq!(decode::exif_details(b"not a picture"), decode::Details::default());
}

#[test]
fn info_fields_live_beside_the_edits_in_the_sidecar() {
    let info = sidecar::Info {
        title: "Bay".into(),
        caption: "A \"quoted\" <caption>\nline two".into(),
        labels: vec!["sea".into(), " Sea ".into(), "holiday".into()],
        licence: "CC BY 4.0".into(),
        ..Default::default()
    }
    .cleaned()
    .unwrap();
    assert_eq!(info.labels, vec!["sea", "holiday"], "trimmed, no repeats");
    // No sidecar: a fresh one; nothing to save: no change.
    let Ok(sidecar::Merged::Write(one)) = sidecar::merge_info(None, &info) else { panic!() };
    assert_eq!(sidecar::read_info(&one), info);
    assert_eq!(sidecar::merge_info(None, &sidecar::Info::default()), Ok(sidecar::Merged::Same));
    // With edits in it: both kept, each readable, either can change without the other.
    let g = sidecar::Geo { width: 40, height: 30, orientation: 1 };
    let edits: model::Edits = serde_json::from_str(r#"{"exposure": 1.0}"#).unwrap();
    let Ok(sidecar::Merged::Write(two)) = sidecar::merge(Some(&one), Some(&edits), &g) else { panic!() };
    assert_eq!((sidecar::read_info(&two), sidecar::read(&two).unwrap().edits), (info.clone(), edits.clone()));
    let less = sidecar::Info { labels: vec!["sea".into()], ..info.clone() };
    let Ok(sidecar::Merged::Write(three)) = sidecar::merge_info(Some(&two), &less) else { panic!() };
    assert_eq!((sidecar::read_info(&three), sidecar::read(&three).unwrap().edits), (less, edits));
    // Taking the edits out keeps the info; taking the info out of an info-only sidecar deletes it.
    let Ok(sidecar::Merged::Write(four)) = sidecar::merge(Some(&three), None, &g) else { panic!() };
    assert_eq!(sidecar::read_info(&four).title, "Bay");
    assert_eq!(sidecar::merge_info(Some(&four), &sidecar::Info::default()), Ok(sidecar::Merged::Delete));
    // Another app's fields stay byte for byte.
    let foreign = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="4"/></rdf:RDF></x:xmpmeta>"#;
    let Ok(sidecar::Merged::Write(five)) = sidecar::merge_info(Some(foreign), &info) else { panic!() };
    assert!(String::from_utf8_lossy(&five).contains(r#"xmp:Rating="4""#));
    assert_eq!(sidecar::merge_info(Some(&five), &sidecar::Info::default()), Ok(sidecar::Merged::Write(foreign.to_vec())));
    // Too long: refused.
    assert!(sidecar::Info { title: "x".repeat(2001), ..Default::default() }.cleaned().is_err());
    assert!(sidecar::Info { labels: (0..51).map(|n| n.to_string()).collect(), ..Default::default() }.cleaned().is_err());
}

#[test]
fn keywords_come_from_folders_camera_date_and_labels() {
    let d = decode::Details {
        make: Some("Google".into()),
        model: Some("Pixel 8".into()),
        taken: Some("2024-07-14T19:05:33".into()),
        ..Default::default()
    };
    let i = sidecar::Info { labels: vec!["holiday".into(), "google".into()], ..Default::default() };
    assert_eq!(info::keywords("ann/Trips/Cornwall/a.jpg", &d, &i), vec!["Trips", "Cornwall", "Google", "Pixel 8", "2024", "July", "holiday"]);
}

async fn put(app: &Shared, id: &str, who: &str, etag: &str, body: &str) -> Got {
    send(app, "PUT", &format!("/api/photos/info/{id}"), Some(who), &[(header::IF_MATCH, etag)], body).await
}

#[tokio::test]
async fn info_is_read_by_viewers_and_saved_by_editors() {
    let root = tmp("info");
    let s = Store { root: root.clone(), keep: 3, spaces: true };
    for (u, n) in [("boss", "Boss"), ("ann", "Ann"), ("bob", "Bob")] {
        auth::add(&s, u, n, &format!("{u}password")).unwrap();
    }
    std::fs::create_dir_all(s.full("ann/Trip")).unwrap();
    std::fs::write(s.full("ann/Trip/a.jpg"), with_exif(&jpeg(&halves(40, 30)), &rich_exif())).unwrap();
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
    let url = format!("/api/photos/info/{a}");
    let g = get(&app, &url, Some("ann"), &[]).await;
    assert_eq!((g.status, g.json["etag"].as_str(), g.json["writable"].as_bool()), (200, Some("none"), Some(true)), "{}", g.json);
    assert_eq!((g.json["exif"]["iso"].as_u64(), g.json["exif"]["lens"].as_str()), (Some(640), Some("Pixel 8 back camera 6.9mm f/1.68")));
    assert_eq!(g.json["info"]["title"], "");
    // Saving: If-Match, valid JSON, small body.
    let body = r#"{"title": "Bay", "labels": ["sea"], "artist": "Ann"}"#;
    assert_eq!(send(&app, "PUT", &url, Some("ann"), &[], body).await.status, 428);
    assert_eq!(put(&app, a, "ann", "abc", body).await.status, 409);
    assert_eq!(put(&app, a, "ann", "none", r#"{"nope": 1}"#).await.status, 400);
    assert_eq!(put(&app, a, "ann", "none", &format!("{{\"title\": \"{}\"}}", "x".repeat(17_000))).await.status, 413);
    let g = put(&app, a, "ann", "none", body).await;
    assert_eq!(g.status, 200, "{}", g.json);
    let etag = g.json["etag"].as_str().unwrap().to_string();
    let original = std::fs::read(app.store.full("ann/Trip/a.jpg")).unwrap();
    assert_eq!(original, with_exif(&jpeg(&halves(40, 30)), &rich_exif()), "the picture itself is never changed");
    let g = get(&app, &url, Some("ann"), &[]).await;
    assert_eq!((g.json["info"]["title"].as_str(), g.json["etag"].as_str()), (Some("Bay"), Some(etag.as_str())));
    assert!(g.json["keywords"].as_array().unwrap().iter().any(|k| k == "sea"), "{}", g.json);
    // Bob, a viewer of the folder: reads, can't save. Carol, nobody: can't even read.
    let folder_id = app.store.synced_folders().unwrap().into_iter().find(|(_, p)| p == "ann/Trip").unwrap().0;
    sharing::set(
        &app.store,
        sharing::Item::Folder(&folder_id),
        &sharing::Share { people: vec![("bob".into(), sharing::Role::View)], link: sharing::Role::None, ..Default::default() },
    )
    .unwrap();
    let g = get(&app, &url, Some("bob"), &[]).await;
    assert_eq!((g.status, g.json["writable"].as_bool(), g.json["info"]["title"].as_str()), (200, Some(false), Some("Bay")));
    assert!(g.json["exif"]["lat"].is_null());
    assert_eq!(put(&app, a, "bob", &etag, r#"{"title": "Mine"}"#).await.status, 403);
    assert_eq!(get(&app, &url, None, &[]).await.status / 100, 4);
}
