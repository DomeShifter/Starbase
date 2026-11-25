//! Albums and Places through the router: create, add, rename, remove, delete, who sees what.

use super::http::{Got, get, send};
use super::*;
use crate::{Shared, auth, sharing};

async fn post(app: &Shared, uri: &str, who: &str, body: Value) -> Got {
    send(app, "POST", uri, Some(who), &[], &body.to_string()).await
}

#[tokio::test]
async fn albums_are_virtual_and_show_only_what_the_viewer_may_see() {
    let root = tmp("albums");
    let s = Store { root: root.clone(), keep: 3, spaces: true };
    for (u, n) in [("boss", "Boss"), ("ann", "Ann"), ("bob", "Bob")] {
        auth::add(&s, u, n, &format!("{u}password")).unwrap();
    }
    let put = |p: &str, b: Vec<u8>| {
        std::fs::create_dir_all(s.full(p).parent().unwrap()).unwrap();
        std::fs::write(s.full(p), b).unwrap();
    };
    let london = Some(((51, 30, 0), 'N', (0, 7, 0), 'W'));
    put("ann/Trip/a.jpg", with_exif(&jpeg(&halves(60, 40)), &exif_block(1, "2024:07:14 18:30:05", london)));
    put("ann/Trip/b.jpg", with_exif(&jpeg(&halves(60, 40)), &exif_block(1, "2021:01:01 00:00:00", None)));
    put("ann/Private/c.jpg", jpeg(&halves(60, 40)));
    put("ann/Trip/notes.txt", b"x".to_vec());
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
    let (a, b, c, txt) = (&ids["ann/Trip/a.jpg"], &ids["ann/Trip/b.jpg"], &ids["ann/Private/c.jpg"], &ids["ann/Trip/notes.txt"]);
    for f in ["ann/Trip", "ann/Private"] {
        assert_eq!(send(&app, "POST", &format!("/api/photos/enable?folder={f}"), Some("ann"), &[], "").await.status, 200);
    }
    // Wait until the pictures are read (the place needs the GPS).
    let t = Instant::now();
    while get(&app, "/api/photos/places", Some("ann"), &[]).await.json["places"].as_array().is_none_or(|p| p.is_empty()) {
        assert!(t.elapsed() < Duration::from_secs(20), "never read");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Create, validate, list.
    assert_eq!(post(&app, "/api/photos/albums", "ann", json!({"name": "  "})).await.status, 400);
    assert_eq!(get(&app, "/api/photos/albums", None, &[]).await.status, 401);
    let g = post(&app, "/api/photos/albums", "ann", json!({"name": "Summer"})).await;
    assert_eq!(g.status, 200, "{}", g.json);
    let id = g.json["id"].as_str().unwrap().to_string();
    let url = format!("/api/photos/albums/{id}");
    // Add: pictures I may see only; twice is once; a text file and a stranger's id are ignored.
    let g = post(&app, &format!("{url}/add"), "ann", json!({"files": [a, b, a, c, txt, "nosuchid"]})).await;
    assert_eq!((g.status, g.json["count"].as_u64()), (200, Some(3)), "{}", g.json);
    let g = get(&app, &url, Some("ann"), &[]).await;
    let names: Vec<&str> = g.json["photos"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["a.jpg", "b.jpg", "c.jpg"], "newest first, c has no date: last; {}", g.json);
    assert_eq!(get(&app, "/api/photos/albums", Some("ann"), &[]).await.json["albums"][0]["count"], 3);

    // Rename.
    assert_eq!(post(&app, &format!("{url}/rename"), "ann", json!({"name": "Summer 2024"})).await.json["name"], "Summer 2024");

    // Bob: no album of his own with that id; his own album with Ann's ids shows nothing yet.
    assert_eq!(get(&app, &url, Some("bob"), &[]).await.status, 404);
    assert_eq!(post(&app, &format!("{url}/delete"), "bob", json!({})).await.status, 404);
    let bid = post(&app, "/api/photos/albums", "bob", json!({"name": "Mine"})).await.json["id"].as_str().unwrap().to_string();
    let g = post(&app, &format!("/api/photos/albums/{bid}/add"), "bob", json!({"files": [a, b]})).await;
    assert_eq!(g.json["count"], 0, "Bob can't see Ann's pictures: {}", g.json);

    // Share Trip with Bob (view): now he may add its pictures, but never sees Private's, and no position.
    let folder_id = app.store.synced_folders().unwrap().into_iter().find(|(_, p)| p == "ann/Trip").unwrap().0;
    sharing::set(
        &app.store,
        sharing::Item::Folder(&folder_id),
        &sharing::Share { people: vec![("bob".into(), sharing::Role::View)], link: sharing::Role::None },
    )
    .unwrap();
    let g = post(&app, &format!("/api/photos/albums/{bid}/add"), "bob", json!({"files": [a, b, c]})).await;
    assert_eq!(g.json["count"], 2, "{}", g.json);
    let g = get(&app, &format!("/api/photos/albums/{bid}"), Some("bob"), &[]).await;
    assert_eq!(g.json["photos"].as_array().unwrap().len(), 2);
    assert!(g.json["photos"][0]["gps"].is_null(), "no position for a viewer: {}", g.json);
    assert_eq!(get(&app, "/api/photos/places", Some("bob"), &[]).await.json["places"], json!([]));
    // Ann's album still lists all three for Ann, and Bob's view of an album holding Private's picture hides it.
    assert_eq!(get(&app, &url, Some("ann"), &[]).await.json["photos"].as_array().unwrap().len(), 3);

    // Places: Ann's London picture, named offline; the one without GPS isn't there.
    let g = get(&app, "/api/photos/places", Some("ann"), &[]).await;
    let places = g.json["places"].as_array().unwrap();
    assert_eq!(places.len(), 1, "{}", g.json);
    let name = places[0]["name"].as_str().unwrap().to_string();
    assert!(name.starts_with("London"), "{name}");
    let g = get(&app, &format!("/api/photos/places?place={}", name.replace(' ', "%20").replace(',', "%2C")), Some("ann"), &[]).await;
    assert_eq!((g.status, g.json["photos"].as_array().map(Vec::len)), (200, Some(1)), "{}", g.json);
    assert_eq!(get(&app, "/api/photos/places?place=Nowhere", Some("ann"), &[]).await.status, 404);

    // Remove, then delete: the files stay.
    let g = post(&app, &format!("{url}/remove"), "ann", json!({"files": [b]})).await;
    assert_eq!(g.json["count"], 2);
    assert_eq!(post(&app, &format!("{url}/delete"), "ann", json!({})).await.json["deleted"], true);
    assert_eq!(get(&app, &url, Some("ann"), &[]).await.status, 404);
    assert!(app.store.full("ann/Trip/a.jpg").is_file() && app.store.full("ann/Trip/b.jpg").is_file());
}
