//! The edits API through the router: who may save, etags, sidecars on disk, thumbnails' `v`.

use super::http::{Got, ask, get, send};
use super::*;
use crate::{Shared, auth, sharing};
use axum::http::header;

async fn put(app: &Shared, id: &str, who: &str, etag: &str, body: &str) -> Got {
    send(app, "PUT", &format!("/api/photos/edits/{id}"), Some(who), &[(header::IF_MATCH, etag)], body).await
}

/// The picture's `v` in its folder's listing, once it is read.
async fn listed_v(app: &Shared, name: &str) -> String {
    let t = Instant::now();
    loop {
        let g = get(app, "/api/photos?folder=ann/Trip", Some("ann"), &[]).await;
        let v = g.json["photos"].as_array().unwrap().iter().find(|p| p["name"] == name).and_then(|p| p["v"].as_str().map(str::to_string));
        if let Some(v) = v {
            return v;
        }
        assert!(t.elapsed() < Duration::from_secs(20), "never read: {}", g.json);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn edits_are_saved_in_a_sidecar_by_people_who_may_edit() {
    let root = tmp("edits");
    let s = Store { root: root.clone(), keep: 3, spaces: true };
    // Boss first: the first account runs the server (sees everything); Ann and Bob are people.
    for (u, n) in [("boss", "Boss"), ("ann", "Ann"), ("bob", "Bob")] {
        auth::add(&s, u, n, &format!("{u}password")).unwrap();
    }
    let put_file = |p: &str, b: &[u8]| {
        std::fs::create_dir_all(s.full(p).parent().unwrap()).unwrap();
        std::fs::write(s.full(p), b).unwrap();
    };
    put_file("ann/Trip/a.jpg", &jpeg(&halves(60, 40)));
    put_file("ann/Trip/b.jpg", &jpeg(&halves(60, 40)));
    put_file("ann/Trip/c.jpg", &jpeg(&halves(60, 40)));
    put_file("ann/Trip/raw.cr2", b"raw");
    put_file("ann/Trip/notes.txt", b"text");
    put_file("bob/secret.txt", b"bob's");
    std::os::unix::fs::symlink(s.full("bob/secret.txt"), s.full("ann/Trip/b.jpg.xmp")).unwrap();
    put_file(
        "ann/Trip/c.jpg.xmp",
        br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:darktable="http://darktable.sf.net/"><darktable:history><rdf:Seq><rdf:li/></rdf:Seq></darktable:history></rdf:Description></rdf:RDF></x:xmpmeta>"#,
    );
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
    let (a, b, c) = (&ids["ann/Trip/a.jpg"], &ids["ann/Trip/b.jpg"], &ids["ann/Trip/c.jpg"]);
    let url = |id: &str| format!("/api/photos/edits/{id}");
    assert_eq!(ask(&app, "POST", "/api/photos/enable?folder=ann/Trip", Some("ann"), &[]).await.status, 200);
    let v0 = listed_v(&app, "a.jpg").await;

    // No sidecar: no change, writable.
    let g = get(&app, &url(a), Some("ann"), &[]).await;
    assert_eq!((g.status, g.json["etag"].as_str(), g.json["writable"].as_bool()), (200, Some("none"), Some(true)), "{}", g.json);
    assert_eq!(
        (g.json["edits"]["exposure"].as_f64(), g.json["foreign"].clone(), g.json["thumb_v"].as_str()),
        (Some(0.0), json!([]), Some(v0.as_str()))
    );

    // Saving needs If-Match, valid edits, a small body.
    let body = r#"{"exposure": 1.0, "crop": {"l": 0.1, "t": 0, "r": 1, "b": 1}}"#;
    assert_eq!(send(&app, "PUT", &url(a), Some("ann"), &[], body).await.status, 428);
    assert_eq!(put(&app, a, "ann", "abc", body).await.status, 409, "wrong etag");
    assert_eq!(put(&app, a, "ann", "none", r#"{"exposure": 99}"#).await.status, 400);
    assert_eq!(put(&app, a, "ann", "none", &format!("{{\"aspect\": \"{}\"}}", "x".repeat(17_000))).await.status, 413);
    let g = put(&app, a, "ann", "none", body).await;
    assert_eq!(g.status, 200, "{}", g.json);
    let etag = g.json["etag"].as_str().unwrap().to_string();
    let v1 = g.json["thumb_v"].as_str().unwrap().to_string();
    assert_ne!(v1, v0, "the thumbnails' key changes with the edits");
    let side = std::fs::read_to_string(app.store.full("ann/Trip/a.jpg.xmp")).unwrap();
    assert!(side.contains(r#"crs:Exposure2012="+1.00""#) && side.contains("starbase:Edits="), "{side}");
    assert_eq!(listed_v(&app, "a.jpg").await, v1, "the listing has the new v");
    // A second save from the same starting point loses: 409 with the current etag.
    let g = put(&app, a, "ann", "none", r#"{"contrast": 5}"#).await;
    assert_eq!((g.status, g.json["etag"].as_str()), (409, Some(etag.as_str())));
    let g = get(&app, &url(a), Some("ann"), &[]).await;
    assert_eq!((g.json["edits"]["exposure"].as_f64(), g.json["source"].as_str()), (Some(1.0), Some("starbase")), "{}", g.json);
    // The thumbnail under the new v is made.
    let t = Instant::now();
    loop {
        let g = get(&app, &format!("/api/photos/thumb/{a}/256?v={v1}"), Some("ann"), &[]).await;
        if g.status == 200 {
            assert_eq!(g.headers[header::ETAG].to_str().unwrap(), format!("\"{v1}-256\""));
            break;
        }
        assert!(t.elapsed() < Duration::from_secs(20), "thumbnail never made: {}", g.status);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Bob, sharing the folder as a viewer: reads, can't save.
    let folder_id = app.store.synced_folders().unwrap().into_iter().find(|(_, p)| p == "ann/Trip").unwrap().0;
    sharing::set(
        &app.store,
        sharing::Item::Folder(&folder_id),
        &sharing::Share { people: vec![("bob".into(), sharing::Role::View)], link: sharing::Role::None },
    )
    .unwrap();
    let g = get(&app, &url(a), Some("bob"), &[]).await;
    assert_eq!((g.status, g.json["writable"].as_bool()), (200, Some(false)));
    assert!(g.json["reason"].as_str().unwrap().contains("not edit"), "{}", g.json);
    assert_eq!(put(&app, a, "bob", &etag, body).await.status, 403);
    assert_eq!(ask(&app, "DELETE", &url(a), Some("bob"), &[(header::IF_MATCH, &etag)]).await.status, 403);
    assert_eq!(get(&app, &url(a), None, &[]).await.status, 401);
    assert_eq!(get(&app, "/api/photos/edits/nosuchid", Some("ann"), &[]).await.status, 403, "no hint that it doesn't exist");
    assert_eq!(get(&app, "/api/photos/edits/nosuchid", Some("boss"), &[]).await.status, 404, "the server's owner sees all");
    assert_eq!(get(&app, &url(&ids["ann/Trip/notes.txt"]), Some("ann"), &[]).await.status, 404, "not a picture");
    let g = get(&app, &url(&ids["ann/Trip/raw.cr2"]), Some("ann"), &[]).await;
    assert_eq!((g.status, g.json["writable"].as_bool()), (200, Some(false)), "RAW: not yet");

    // A sidecar that is a link to Bob's file: never read or written through.
    let g = get(&app, &url(b), Some("ann"), &[]).await;
    assert_eq!(g.json["writable"].as_bool(), Some(false), "{}", g.json);
    assert_eq!(put(&app, b, "ann", "*", body).await.status, 409);
    assert_eq!(std::fs::read(app.store.full("bob/secret.txt")).unwrap(), b"bob's");

    // darktable's sidecar: edits allowed, flagged; darktable's history kept.
    let g = get(&app, &url(c), Some("ann"), &[]).await;
    assert_eq!((g.json["foreign"].clone(), g.json["writable"].as_bool()), (json!(["darktable"]), Some(true)), "{}", g.json);
    let ce = g.json["etag"].as_str().unwrap().to_string();
    let g = put(&app, c, "ann", &ce, r#"{"rot90": 1}"#).await;
    assert_eq!(g.status, 200, "{}", g.json);
    assert!(std::fs::read_to_string(app.store.full("ann/Trip/c.jpg.xmp")).unwrap().contains("<darktable:history>"));
    let g = ask(&app, "DELETE", &url(c), Some("ann"), &[(header::IF_MATCH, g.json["etag"].as_str().unwrap())]).await;
    assert_eq!(g.status, 200);
    assert!(app.store.full("ann/Trip/c.jpg.xmp").is_file(), "darktable's sidecar stays");

    // Reset: our sidecar goes, and the picture's v is the original again.
    let g = ask(&app, "DELETE", &url(a), Some("ann"), &[(header::IF_MATCH, &etag)]).await;
    assert_eq!((g.status, g.json["etag"].as_str(), g.json["thumb_v"].as_str()), (200, Some("none"), Some(v0.as_str())), "{}", g.json);
    assert!(!app.store.full("ann/Trip/a.jpg.xmp").exists());

    // A sidecar written by another app is noticed by the next scan: new v.
    let e: super::model::Edits = serde_json::from_str(r#"{"saturation": -40}"#).unwrap();
    let super::sidecar::Merged::Write(x) = super::sidecar::merge(None, Some(&e)).unwrap() else { panic!() };
    std::fs::write(app.store.full("ann/Trip/a.jpg.xmp"), x).unwrap();
    let svc = service(&app.store).unwrap();
    let t = Instant::now();
    loop {
        let ticket = svc.request_scan("ann/Trip", false, true);
        svc.wait_scan(ticket, Duration::from_secs(10));
        let v = listed_v(&app, "a.jpg").await;
        if v != v0 && v != v1 {
            break;
        }
        assert!(t.elapsed() < Duration::from_secs(20), "the new sidecar wasn't noticed");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    std::fs::remove_file(app.store.full("ann/Trip/a.jpg.xmp")).unwrap();

    // A folder the server can't write in (not testable as root, who may write anywhere).
    // SAFETY: geteuid only reads this process's user id.
    if unsafe { libc::geteuid() } != 0 {
        use std::os::unix::fs::PermissionsExt;
        let dir = app.store.full("ann/Trip");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let g = get(&app, &url(a), Some("ann"), &[]).await;
        assert_eq!(g.json["writable"].as_bool(), Some(false), "{}", g.json);
        let g = put(&app, a, "ann", "none", body).await;
        assert_eq!(g.status, 403, "{}", g.json);
        assert!(g.json["error"].as_str().unwrap().contains("can't be saved"), "{}", g.json);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    if let Some(svc) = service(&app.store) {
        svc.stop();
    }
    let _ = std::fs::remove_dir_all(&root);
}
