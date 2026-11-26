//! The Locked folder through the router: hidden everywhere, password to open, move in and out.

use super::http::{Got, get, send};
use super::*;
use crate::{Shared, auth, sharing};

async fn post(app: &Shared, uri: &str, who: &str, body: Value) -> Got {
    send(app, "POST", uri, Some(who), &[], &body.to_string()).await
}

fn count(g: &Got, key: &str) -> usize {
    g.json[key].as_array().map_or(0, Vec::len)
}

#[tokio::test]
async fn locked_pictures_are_hidden_until_the_owner_types_the_password() {
    let root = tmp("locked");
    let s = Store { root: root.clone(), keep: 3, spaces: true };
    for (u, n) in [("boss", "Boss"), ("ann", "Ann"), ("bob", "Bob")] {
        auth::add(&s, u, n, &format!("{u}password")).unwrap();
    }
    for p in ["ann/Trip/a.jpg", "ann/Trip/b.jpg"] {
        std::fs::create_dir_all(s.full(p).parent().unwrap()).unwrap();
        std::fs::write(s.full(p), jpeg(&halves(60, 40))).unwrap();
    }
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
    let (a, b) = (&ids["ann/Trip/a.jpg"], &ids["ann/Trip/b.jpg"]);
    assert_eq!(send(&app, "POST", "/api/photos/enable?folder=ann/Trip", Some("ann"), &[], "").await.status, 200);
    let folder_id = app.store.synced_folders().unwrap().into_iter().find(|(_, p)| p == "ann/Trip").unwrap().0;
    sharing::set(
        &app.store,
        sharing::Item::Folder(&folder_id),
        &sharing::Share { people: vec![("bob".into(), sharing::Role::View)], link: sharing::Role::None, ..Default::default() },
    )
    .unwrap();
    let t = Instant::now();
    while count(&get(&app, "/api/photos?folder=ann/Trip", Some("ann"), &[]).await, "photos") < 2 {
        assert!(t.elapsed() < Duration::from_secs(20), "never listed");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(get(&app, "/api/photos/locked", None, &[]).await.status, 401);

    // Move a.jpg in: everyone loses it everywhere; a stranger's id and a repeat change nothing.
    assert_eq!(post(&app, "/api/photos/locked/add", "bob", json!({"files": [a]})).await.json["changed"], 0, "not Bob's");
    assert_eq!(post(&app, "/api/photos/locked/add", "ann", json!({"files": [a, a, "nosuchid"]})).await.json["changed"], 1);
    for who in ["ann", "bob", "boss"] {
        let g = get(&app, "/api/photos?folder=ann/Trip", Some(who), &[]).await;
        assert!(!g.json.to_string().contains(a.as_str()), "{who}: {}", g.json);
    }
    assert_eq!(count(&get(&app, "/api/photos?folder=ann/Trip", Some("ann"), &[]).await, "photos"), 1);
    assert_eq!(get(&app, &format!("/api/photos/thumb/{a}/256"), Some("ann"), &[]).await.status, 403, "no thumbnail while locked");
    let boss = auth::check(&app.store, "boss", "bosspassword").unwrap();
    assert_eq!(sharing::file_role(&app.store, true, Some(&boss), a), sharing::Role::None, "not even the server's owner");
    let drive = get(&app, "/api/files", Some("ann"), &[]).await;
    assert!(!drive.json.to_string().contains(a.as_str()), "Drive list: {}", drive.json);
    for u in ["/api/photos/albums", "/api/photos/places", "/api/photos/events"] {
        assert!(!get(&app, u, Some("ann"), &[]).await.json.to_string().contains(a.as_str()), "{u}");
    }
    assert!(crate::photos::locked::is_locked(&app.store, a), "a public link looks here too");

    // Closed: no list, no move out. Wrong password refused; right one opens.
    assert_eq!(get(&app, "/api/photos/locked", Some("ann"), &[]).await.json["unlocked"], false);
    assert_eq!(post(&app, "/api/photos/locked/remove", "ann", json!({"files": [a]})).await.status, 403);
    assert_eq!(post(&app, "/api/photos/locked/unlock", "ann", json!({"password": "nope"})).await.status, 401);
    assert_eq!(post(&app, "/api/photos/locked/unlock", "bob", json!({"password": "annpassword"})).await.status, 401);
    assert_eq!(post(&app, "/api/photos/locked/unlock", "ann", json!({"password": "annpassword"})).await.status, 200);
    let g = get(&app, "/api/photos/locked", Some("ann"), &[]).await;
    assert_eq!((g.json["unlocked"].as_bool(), count(&g, "photos")), (Some(true), 1), "{}", g.json);
    assert_ne!(get(&app, &format!("/api/photos/thumb/{a}/256"), Some("ann"), &[]).await.status, 403);
    // Even open, it isn't in the normal library; Bob still sees nothing and can't open Locked.
    assert_eq!(count(&get(&app, "/api/photos?folder=ann/Trip", Some("ann"), &[]).await, "photos"), 1);
    assert_eq!(count(&get(&app, "/api/photos?folder=ann/Trip", Some("bob"), &[]).await, "photos"), 1);
    assert_eq!(get(&app, "/api/photos/locked", Some("bob"), &[]).await.json["unlocked"], false);

    // Relock, then open again and move out.
    assert_eq!(post(&app, "/api/photos/locked/lock", "ann", json!({})).await.json["unlocked"], false);
    assert_eq!(get(&app, "/api/photos/locked", Some("ann"), &[]).await.json["unlocked"], false);
    assert_eq!(post(&app, "/api/photos/locked/unlock", "ann", json!({"password": "annpassword"})).await.status, 200);
    assert_eq!(post(&app, "/api/photos/locked/remove", "ann", json!({"files": [a, b]})).await.json["changed"], 1);
    assert_eq!(count(&get(&app, "/api/photos?folder=ann/Trip", Some("bob"), &[]).await, "photos"), 2);
}
