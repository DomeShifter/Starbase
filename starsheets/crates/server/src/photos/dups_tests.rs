//! The Duplicates list through the router: two identical pictures and a different one.

use super::http::{get, send};
use super::*;
use crate::{Shared, auth};

#[tokio::test]
async fn identical_pictures_form_one_group() {
    let root = tmp("dups");
    let s = Store { root: root.clone(), keep: 3, spaces: true };
    for (u, n) in [("boss", "Boss"), ("ann", "Ann"), ("bob", "Bob")] {
        auth::add(&s, u, n, &format!("{u}password")).unwrap();
    }
    let same = jpeg(&halves(60, 40));
    for (p, b) in [("ann/Trip/a.jpg", same.clone()), ("ann/Trip/Copy/a2.jpg", same), ("ann/Trip/b.jpg", jpeg(&halves(40, 60)))] {
        std::fs::create_dir_all(s.full(p).parent().unwrap()).unwrap();
        std::fs::write(s.full(p), b).unwrap();
    }
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
    assert_eq!(get(&app, "/api/photos/duplicates?folder=ann/Trip", None, &[]).await.status, 401);
    assert_eq!(send(&app, "POST", "/api/photos/enable?folder=ann/Trip", Some("ann"), &[], "").await.status, 200);
    let t = Instant::now();
    loop {
        let g = get(&app, "/api/photos/duplicates?folder=ann/Trip", Some("ann"), &[]).await;
        assert_eq!(g.status, 200, "{}", g.json);
        let groups = g.json["groups"].as_array().unwrap();
        if !groups.is_empty() {
            assert_eq!(groups.len(), 1, "{}", g.json);
            assert_eq!(groups[0]["photos"].as_array().unwrap().len(), 2, "{}", g.json);
            assert_eq!(groups[0]["wasted"], groups[0]["photos"][0]["size"]);
            break;
        }
        assert!(t.elapsed() < Duration::from_secs(20), "never grouped");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // Someone with no access is refused.
    assert_eq!(get(&app, "/api/photos/duplicates?folder=ann/Trip", Some("bob"), &[]).await.status, 403);
}
