//! People through the router: names are per account, never shown to anyone else.

use super::http::{Got, get, send};
use super::*;
use crate::photos::faces::{self, Found};
use crate::{Shared, auth};

async fn post(app: &Shared, uri: &str, who: &str, body: Value) -> Got {
    send(app, "POST", uri, Some(who), &[], &body.to_string()).await
}

fn face(e: [f32; 4]) -> Found {
    Found { x: 0.1, y: 0.1, w: 0.2, h: 0.2, score: 0.95, px: 80.0, emb: e.to_vec() }
}

#[tokio::test]
async fn a_name_is_seen_only_by_the_account_that_gave_it() {
    let root = tmp("people");
    let s = Store { root: root.clone(), keep: 3, spaces: true };
    for (u, n) in [("boss", "Boss"), ("ada", "Ada"), ("bob", "Bob")] {
        auth::add(&s, u, n, &format!("{u}password")).unwrap();
    }
    // Ada and Bob each have their own copies of the same two pictures (same content, same faces).
    for p in ["ada/Pics/a.jpg", "bob/Mine/a.jpg", "ada/Pics/b.jpg", "bob/Mine/b.jpg"] {
        std::fs::create_dir_all(s.full(p).parent().unwrap()).unwrap();
        let img = if p.ends_with("a.jpg") { halves(60, 40) } else { halves(40, 60) };
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
    let ph = service(&app.store).unwrap();
    let t = Instant::now();
    while ph.with_db(faces::thumb_paths).unwrap_or_default().len() < 4 {
        assert!(t.elapsed() < Duration::from_secs(20), "never read");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // One person's face in both pictures (as the face worker would keep them), then grouped.
    let model = ph.face_model().to_string();
    let thumbs: std::collections::BTreeSet<String> = ph.with_db(faces::thumb_paths).unwrap().into_iter().map(|(t, _)| t).collect();
    assert_eq!(thumbs.len(), 2, "same content, same thumbnail key");
    for (i, t) in thumbs.iter().enumerate() {
        ph.with_db(|ix| faces::keep(ix, t, &model, &[face([1.0, 0.02 * i as f32, 0.0, 0.0])])).unwrap();
    }
    faces::regroup(ph.db_lock(), &model).unwrap();

    let ada = get(&app, "/api/photos/people", Some("ada"), &[]).await.json;
    let g = ada["groups"][0]["id"].as_str().expect("one group").to_string();
    // The cover face as its own sharp picture (cut from the 1024 thumbnail), only for who may see it.
    let cover = &ada["groups"][0]["cover"];
    let url = format!("/api/photos/people/face/{}/{}", cover["face"], cover["id"].as_str().unwrap());
    let pic = get(&app, &url, Some("ada"), &[]).await;
    assert_eq!(pic.status, 200, "{url}");
    assert_eq!(image::load_from_memory(&pic.bytes).unwrap().width(), 256);
    assert_eq!(get(&app, &url, Some("bob"), &[]).await.status, 403, "not Bob's picture");
    assert_eq!(get(&app, &format!("/api/photos/people/face/999999/{}", cover["id"].as_str().unwrap()), Some("ada"), &[]).await.status, 404);
    let named = post(&app, &format!("/api/photos/people/{g}/name"), "ada", json!({"name": "Sally Ride"})).await;
    assert_eq!(named.status, 200);
    let p = named.json["id"].as_str().unwrap().to_string();
    let ada = get(&app, "/api/photos/people", Some("ada"), &[]).await.json;
    assert_eq!(ada["people"][0]["name"], "Sally Ride");

    // Bob sees the faces in his own pictures, but never Ada's name for them.
    let bob = get(&app, "/api/photos/people", Some("bob"), &[]).await.json;
    assert!(!bob.to_string().contains("Sally Ride"), "{bob}");
    assert_eq!(bob["people"], json!([]));
    assert_eq!(bob["groups"][0]["id"], p.as_str());
    assert_eq!(bob["groups"][0]["name"], "");
    let one = get(&app, &format!("/api/photos/people/{p}"), Some("bob"), &[]).await.json;
    assert_eq!(one["name"], "");
    assert!(!one.to_string().contains("Sally Ride"));

    let bob_pic = one["photos"][0]["id"].as_str().unwrap().to_string();
    let unnamed = get(&app, &format!("/api/photos/faces/{bob_pic}"), Some("bob"), &[]).await.json;
    assert_eq!((unnamed["people"].clone(), unnamed["unnamed"].clone()), (json!([]), json!(1)), "{unnamed}");
    // Bob's own name is his alone.
    assert_eq!(post(&app, &format!("/api/photos/people/{p}/name"), "bob", json!({"name": "Mae"})).await.status, 200);
    assert_eq!(get(&app, "/api/photos/people", Some("bob"), &[]).await.json["people"][0]["name"], "Mae");
    assert_eq!(get(&app, "/api/photos/people", Some("ada"), &[]).await.json["people"][0]["name"], "Sally Ride");

    // The viewer's Info panel: the faces in one picture, with the asker's own names only.
    let ada_pic = get(&app, &format!("/api/photos/people/{p}"), Some("ada"), &[]).await.json["photos"][0]["id"].as_str().unwrap().to_string();
    let mine = get(&app, &format!("/api/photos/faces/{ada_pic}"), Some("ada"), &[]).await;
    assert_eq!(mine.status, 200);
    assert_eq!(mine.json["people"][0]["name"], "Sally Ride", "{}", mine.json);
    assert_eq!(mine.json["people"][0]["id"], p.as_str());
    assert_eq!(mine.json["people"][0]["pic"], ada_pic.as_str());
    assert_eq!(mine.json["unnamed"], 0);
    let his = get(&app, &format!("/api/photos/faces/{bob_pic}"), Some("bob"), &[]).await.json;
    assert_eq!(his["people"][0]["name"], "Mae");
    assert!(!his.to_string().contains("Sally Ride"), "{his}");
    // Someone else's picture, or none: not found (nothing about its faces).
    for (pic, who) in [(&ada_pic, "bob"), (&bob_pic, "ada")] {
        let r = get(&app, &format!("/api/photos/faces/{pic}"), Some(who), &[]).await;
        assert_eq!(r.status, 404, "{who} asking for {pic}");
        assert!(!r.json.to_string().contains("Mae") && !r.json.to_string().contains("Sally"));
    }
    assert_eq!(get(&app, "/api/photos/faces/nope", Some("ada"), &[]).await.status, 404);
    assert_eq!(get(&app, &format!("/api/photos/faces/{ada_pic}"), None, &[]).await.status, 401);
    // Ignore (a stranger), then every way back. Nothing is ever deleted.
    let list = |who: &'static str| {
        let app = app.clone();
        async move { get(&app, "/api/photos/people", Some(who), &[]).await.json }
    };
    assert_eq!(post(&app, &format!("/api/photos/people/{p}/ignore"), "ada", json!({})).await.status, 200);
    let v = list("ada").await;
    assert_eq!((v["people"].clone(), v["groups"].clone(), v["ignored_count"].clone()), (json!([]), json!([]), json!(2)), "{v}");
    let ig: Vec<i64> = v["ignored"].as_array().unwrap().iter().map(|f| f["face"].as_i64().unwrap()).collect();
    assert_eq!(post(&app, "/api/photos/people/marked/isface", "ada", json!({"faces": [ig[0]]})).await.status, 404, "not on that list");
    assert_eq!(post(&app, "/api/photos/people/marked/unignore", "ada", json!({"faces": [999_999]})).await.status, 404);
    assert_eq!(post(&app, "/api/photos/people/marked/unignore", "ada", json!({"faces": ig})).await.status, 200);
    let v = list("ada").await;
    assert_eq!((v["ignored_count"].clone(), v["groups"].as_array().unwrap().len()), (json!(0), 1), "back as one unnamed group: {v}");
    // Not a face, from a group; then "It is a face"; then assign one to a person.
    let g = v["groups"][0]["id"].as_str().unwrap().to_string();
    let one = get(&app, &format!("/api/photos/people/{g}"), Some("ada"), &[]).await.json;
    let f0 = one["faces"][0]["face"].as_i64().unwrap();
    assert_eq!(post(&app, &format!("/api/photos/people/{g}/notface"), "ada", json!({"faces": [f0]})).await.status, 200);
    let v = list("ada").await;
    assert_eq!(v["notfaces"][0]["face"], f0);
    assert_eq!(post(&app, "/api/photos/people/marked/isface", "ada", json!({"faces": [f0]})).await.status, 200);
    assert_eq!(list("ada").await["notfaces_count"], 0);
    assert_eq!(post(&app, &format!("/api/photos/people/{g}/ignore"), "ada", json!({"faces": [f0]})).await.status, 200);
    let named = post(&app, &format!("/api/photos/people/{g}/name"), "ada", json!({"name": "Kim"})).await.json["id"].as_str().unwrap().to_string();
    assert_eq!(post(&app, "/api/photos/people/marked/assign", "ada", json!({"faces": [f0], "into": named})).await.status, 200);
    let v = list("ada").await;
    assert_eq!((v["ignored_count"].clone(), v["people"][0]["count"].clone()), (json!(0), json!(2)), "{v}");
    // The group page path (verify of #183): ignore one face of a two-face group, then "Not a face"
    // for the faces still on the page, by id (the group itself may be gone after the regroup).
    let v = list("ada").await;
    let pid = v["people"][0]["id"].as_str().unwrap().to_string();
    let page = get(&app, &format!("/api/photos/people/{pid}"), Some("ada"), &[]).await.json;
    let ids: Vec<i64> = page["faces"].as_array().unwrap().iter().map(|f| f["face"].as_i64().unwrap()).collect();
    assert_eq!(ids.len(), 2);
    assert_eq!(post(&app, "/api/photos/people/faces/ignore", "ada", json!({"faces": [ids[0]]})).await.status, 200);
    faces::regroup(ph.db_lock(), &model).unwrap();
    assert_eq!(post(&app, "/api/photos/people/faces/notface", "ada", json!({"faces": [ids[1]]})).await.status, 200);
    assert_eq!(post(&app, "/api/photos/people/faces/notface", "ada", json!({"faces": [ids[1]]})).await.status, 404, "already marked");
    let v = list("ada").await;
    assert_eq!((v["ignored_count"].clone(), v["notfaces_count"].clone()), (json!(1), json!(1)), "{v}");
    assert!(v["ignored"][0]["grp"].as_i64().is_some(), "marked faces say their group: {v}");
    let _ = std::fs::remove_dir_all(root);
}
