//! "Scan now" through the router: signed in only, once per 30 s, only my own folders.

use super::http::{get, send};
use super::*;
use crate::photos::faces::{self, Found};
use crate::{Shared, auth};

#[tokio::test]
async fn scan_now_queues_only_my_folders_and_waits_between_asks() {
    let root = tmp("scan-now");
    let s = Store { root: root.clone(), keep: 3, spaces: true };
    for (u, n) in [("boss", "Boss"), ("ada", "Ada"), ("bob", "Bob")] {
        auth::add(&s, u, n, &format!("{u}password")).unwrap();
    }
    for p in ["ada/Pics/a.jpg", "bob/Mine/b.jpg"] {
        std::fs::create_dir_all(s.full(p).parent().unwrap()).unwrap();
        let img = if p.starts_with("ada") { halves(40, 20) } else { halves(30, 20) };
        std::fs::write(s.full(p), jpeg(&img)).unwrap();
    }
    s.synced_index().unwrap();
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
    for (u, f) in [("ada", "ada/Pics"), ("bob", "bob/Mine")] {
        assert_eq!(send(&app, "POST", &format!("/api/photos/enable?folder={f}"), Some(u), &[], "").await.status, 200);
    }

    assert_eq!(send(&app, "POST", "/api/photos/scan", None, &[], "").await.status, 401, "signed out");
    assert_eq!(get(&app, "/api/photos/scan", None, &[]).await.status, 401, "signed out");

    // A new picture, then "Scan now": it is found.
    std::fs::write(app.store.full("ada/Pics/c.jpg"), jpeg(&halves(20, 40))).unwrap();
    let ada = send(&app, "POST", "/api/photos/scan", Some("ada"), &[], "").await;
    assert_eq!(ada.status, 200);
    assert_eq!(ada.json["queued"], 1, "only Ada's own folder");
    let again = send(&app, "POST", "/api/photos/scan", Some("ada"), &[], "").await;
    assert_eq!(again.status, 429, "a second ask within 30 s");
    assert!(again.json["error"].as_str().unwrap_or_default().contains("seconds"), "{}", again.json);

    let bob = send(&app, "POST", "/api/photos/scan", Some("bob"), &[], "").await;
    assert_eq!(bob.status, 200, "the gap is per person");
    assert_eq!(bob.json["queued"], 1, "only Bob's own folder");
    let boss = send(&app, "POST", "/api/photos/scan", Some("boss"), &[], "").await;
    assert_eq!(boss.json["queued"], 2, "the owner: every folder with Photos on");

    let t = Instant::now();
    loop {
        let st = get(&app, "/api/photos/scan", Some("ada"), &[]).await.json;
        if st["busy"] == false {
            assert_eq!(st["queued"], 1);
            break;
        }
        assert!(t.elapsed() < Duration::from_secs(20), "scan never finished: {st}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let ph = service(&app.store).unwrap();
    assert!(ph.rows("ada/Pics", true).iter().any(|r| r.path == "ada/Pics/c.jpg"), "the new picture is in the index");
    // A face in Bob's picture is not one of Ada's new faces; one in hers is.
    while ph.with_db(faces::thumb_paths).unwrap_or_default().len() < 3 {
        assert!(t.elapsed() < Duration::from_secs(20), "thumbnails never made");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let model = ph.face_model().to_string();
    let face = Found { x: 0.1, y: 0.1, w: 0.2, h: 0.2, score: 0.95, px: 80.0, emb: vec![1.0, 0.0, 0.0, 0.0] };
    let thumb_of = |path: &str| ph.with_db(faces::thumb_paths).unwrap().into_iter().find(|(_, p)| p == path).unwrap().0;
    // Thumbnail ids first: with_db is not re-entrant (a lookup inside it waits on itself forever).
    let (bob_t, ada_t) = (thumb_of("bob/Mine/b.jpg"), thumb_of("ada/Pics/c.jpg"));
    ph.with_db(|ix| faces::keep(ix, &bob_t, &model, std::slice::from_ref(&face))).unwrap();
    assert_eq!(get(&app, "/api/photos/scan", Some("ada"), &[]).await.json["faces_new"], 0, "Bob's face isn't counted for Ada");
    ph.with_db(|ix| faces::keep(ix, &ada_t, &model, &[face])).unwrap();
    assert_eq!(get(&app, "/api/photos/scan", Some("ada"), &[]).await.json["faces_new"], 1, "Ada's new face");
    let first = get(&app, "/api/photos/scan", Some("ada"), &[]).await.json;
    assert_eq!(first["photos_new"], 2, "first time: what entered in the last 10 minutes (a and c)");
    // A picture the watcher indexes BEFORE the next click still counts as new.
    tokio::time::sleep(Duration::from_millis(1200)).await;
    std::fs::write(app.store.full("ada/Pics/d.jpg"), jpeg(&halves(50, 30))).unwrap();
    let t = Instant::now();
    while !ph.rows("ada/Pics", true).iter().any(|r| r.path == "ada/Pics/d.jpg") {
        assert!(t.elapsed() < Duration::from_secs(20), "the watcher never indexed d.jpg");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    crate::photos::scan_now::backdate(&ph, "ada");
    assert_eq!(send(&app, "POST", "/api/photos/scan", Some("ada"), &[], "").await.status, 200);
    let t = Instant::now();
    let st = loop {
        let st = get(&app, "/api/photos/scan", Some("ada"), &[]).await.json;
        if st["busy"] == false {
            break st;
        }
        assert!(t.elapsed() < Duration::from_secs(20), "second scan never finished: {st}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(st["photos_new"], 1, "only d.jpg is new since Ada's last Scan now: {st}");
    // Each person sees their own run.
    let none = get(&app, "/api/photos/scan", Some("boss"), &[]).await.json;
    assert_eq!(none["queued"], 2);
    let _ = std::fs::remove_dir_all(root);
}
