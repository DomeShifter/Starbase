//! `GET /api/photos?all=1`: every picture the person may see in the Drive that Photos has read
//! (folders where Photos is off have nothing in the index, so they add nothing). Same reply shape
//! as a folder listing, so Photo Mode shows it like any folder. `sort` and `desc` work as there.

use axum::body::Body;
use axum::http::Response;
use serde_json::{Value, json};

use super::albums::{rows_of, seen};
use super::routes::{iso, mark_stars, photo_json, stack_raw_pairs, stars_of};
use crate::{Shared, auth, blocking, error, send_json, sharing};

pub(super) async fn list_all(app: &Shared, sort: &str, desc: bool, me: Option<auth::User>) -> Response<Body> {
    let stars = stars_of(app, me.as_ref()).await;
    let mine = blocking(app, move |a| {
        let acc = sharing::Access::new(&a.store, a.auth, me.as_ref());
        acc.file_paths.keys().filter_map(|id| seen(a, &acc, id)).collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    let a = app.clone();
    let sort = sort.to_string();
    let got = tokio::task::spawn_blocking(move || {
        let (mut rows, waiting) = rows_of(&a, mine);
        let secs = |r: &super::index::Row| r.mtime_ns.div_euclid(1_000_000_000);
        for (_, r, _) in rows.iter_mut().filter(|(_, r, _)| super::video::is_video(&r.media)) {
            if r.meta.taken.is_none()
                && let Some(i) = super::video::info_of(&a.store, &r.path, r.size, r.mtime_ns)
            {
                r.meta.taken = i.created.map(iso);
            }
        }
        let key = |r: &super::index::Row| match sort.as_str() {
            "name" => r.name.to_lowercase(),
            "modified" => format!("{:020}", secs(r)),
            _ => r.meta.taken.clone().map(|t| t.chars().take(19).collect()).unwrap_or_else(|| iso(secs(r))),
        };
        rows.sort_by(|x, y| (key(&x.1), &x.1.path).cmp(&(key(&y.1), &y.1.path)));
        if desc {
            rows.reverse();
        }
        (rows, waiting)
    })
    .await;
    let Ok((rows, waiting)) = got else { return error(500, "server error") };
    let mut photos: Vec<Value> = rows.iter().map(|(id, r, gps)| photo_json(r, id, *gps)).collect();
    mark_stars(&mut photos, &stars);
    let photos = stack_raw_pairs(photos);
    send_json(
        &json!({"folder": "", "sub": true, "all": true, "role": "view", "enabled": true, "can_enable": false, "can_disable": false, "photos": photos, "waiting": waiting}),
        200,
    )
}
