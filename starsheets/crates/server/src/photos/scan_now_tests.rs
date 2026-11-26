//! "Scan now" through the router: signed in only, once per 30 s, only my own folders.

use super::http::{get, send};
use super::*;
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
        std::fs::write(s.full(p), jpeg(&halves(40, 20))).unwrap();
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
    // Each person sees their own run.
    let none = get(&app, "/api/photos/scan", Some("boss"), &[]).await.json;
    assert_eq!(none["queued"], 2);
    let _ = std::fs::remove_dir_all(root);
}
