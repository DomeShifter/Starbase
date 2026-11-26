//! Events: pictures grouped by when (and where) they were taken. Computed from the index on every
//! request, so nothing is stored and nothing moves (docs/photos-api.md, "Events").
//!
//! - `GET /api/photos/events`: `{"events": [{"id", "name", "count", "cover"}], "waiting"}`, newest first.
//! - `GET /api/photos/events?event=ID`: `{"event", "id", "name", "photos", "waiting"}`.
//!
//! A new event starts when two pictures in a row are 6 hours or more apart, or (when both have a
//! position I may know) more than 50 km apart. Groups of fewer than 5 pictures are not events.
//! "Save as album" is the ordinary create-album call with the event's picture ids.

use std::collections::HashMap;

use axum::body::Body;
use axum::http::Response;
use serde_json::{Value, json};

use super::albums::{Seen, cover, place_name, rows_of, seen};
use super::routes::{mark_stars, photo_json, stars_of};
use crate::{Shared, auth, blocking, error, send_json, sharing};

pub const GAP_SECS: i64 = 6 * 3600;
pub const FAR_KM: f64 = 50.0;
pub const MIN_PICS: usize = 5;

/// "2026-08-03T10:00:00" to (days since 1970-01-01, seconds into the day).
fn parse(t: &str) -> Option<(i64, i64)> {
    let n = |a: usize, b: usize| t.get(a..b)?.parse::<i64>().ok();
    let (y, m, d, h, mi, s) = (n(0, 4)?, n(5, 7)?, n(8, 10)?, n(11, 13).unwrap_or(0), n(14, 16).unwrap_or(0), n(17, 19).unwrap_or(0));
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // days from civil (Howard Hinnant)
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468, h * 3600 + mi * 60 + s))
}

fn km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (p1, p2) = (a.0.to_radians(), b.0.to_radians());
    let (dp, dl) = ((b.0 - a.0).to_radians(), (b.1 - a.1).to_radians());
    let h = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    12_742.0 * h.sqrt().asin()
}

/// Groups pictures (given oldest first as (seconds, position)) into runs; returns index ranges of
/// the runs with at least `min` pictures.
pub fn group(items: &[(i64, Option<(f64, f64)>)], gap: i64, far: f64, min: usize) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut last_pos: Option<(f64, f64)> = None;
    for i in 0..items.len() {
        if i > start {
            let jump = items[i].1.zip(last_pos).is_some_and(|(a, b)| km(a, b) > far);
            if items[i].0 - items[i - 1].0 >= gap || jump {
                if i - start >= min {
                    out.push(start..i);
                }
                start = i;
                last_pos = None;
            }
        }
        if items[i].1.is_some() {
            last_pos = items[i].1;
        }
    }
    if items.len() - start >= min && start < items.len() {
        out.push(start..items.len());
    }
    out
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const DAYS: [&str; 7] = ["Thursday", "Friday", "Saturday", "Sunday", "Monday", "Tuesday", "Wednesday"];

fn ymd(t: &str) -> (i64, usize, i64) {
    let n = |a: usize, b: usize| t.get(a..b).and_then(|x| x.parse::<i64>().ok()).unwrap_or(1);
    (n(0, 4), (n(5, 7).clamp(1, 12) - 1) as usize, n(8, 10))
}

/// "Whitby · 3–5 Aug 2026", or "Sunday 3 Aug 2026" with no place.
pub fn name(place: Option<&str>, first: &str, last: &str) -> String {
    let (a, b) = (ymd(first), ymd(last));
    let dates = if a == b {
        format!("{} {} {}", a.2, MONTHS[a.1], a.0)
    } else if a.0 == b.0 && a.1 == b.1 {
        format!("{}–{} {} {}", a.2, b.2, MONTHS[a.1], a.0)
    } else if a.0 == b.0 {
        format!("{} {} – {} {} {}", a.2, MONTHS[a.1], b.2, MONTHS[b.1], a.0)
    } else {
        format!("{} {} {} – {} {} {}", a.2, MONTHS[a.1], a.0, b.2, MONTHS[b.1], b.0)
    };
    match place {
        Some(p) => format!("{p} · {dates}"),
        None => {
            let wd = parse(first).map_or("", |(d, _)| DAYS[d.rem_euclid(7) as usize]);
            format!("{wd} {dates}")
        }
    }
}

pub async fn events(app: &Shared, me: Option<auth::User>, which: Option<String>) -> Response<Body> {
    if app.auth && me.is_none() {
        return error(401, "Sign in first.");
    }
    let stars = stars_of(app, me.as_ref()).await;
    let mine = blocking(app, move |a| {
        let acc = sharing::Access::new(&a.store, a.auth, me.as_ref());
        acc.file_paths.keys().filter_map(|id| seen(a, &acc, id)).collect::<Vec<Seen>>()
    })
    .await
    .unwrap_or_default();
    let app = app.clone();
    let got = tokio::task::spawn_blocking(move || {
        let (rows, waiting) = rows_of(&app, mine);
        // Oldest first; pictures without a capture time can't be placed in time.
        let mut rows: Vec<_> = rows
            .into_iter()
            .filter_map(|x| {
                let (d, s) = parse(x.1.meta.taken.as_deref()?)?;
                Some((d * 86_400 + s, x))
            })
            .collect();
        rows.sort_by(|a, b| (a.0, &a.1.1.path).cmp(&(b.0, &b.1.1.path)));
        // A position only counts for people who may know it (the `gps` flag, as in Places).
        let pos = |x: &(String, super::index::Row, bool)| x.2.then(|| x.1.meta.lat.zip(x.1.meta.lon)).flatten();
        let items: Vec<(i64, Option<(f64, f64)>)> = rows.iter().map(|(t, x)| (*t, pos(x))).collect();
        let mut cache: HashMap<(i64, i64), String> = HashMap::new();
        let mut out = Vec::new();
        for r in group(&items, GAP_SECS, FAR_KM, MIN_PICS) {
            let run = &rows[r];
            let taken = |i: usize| run[i].1.1.meta.taken.clone().unwrap_or_default().chars().take(19).collect::<String>();
            let place = run.iter().find_map(|(_, x)| pos(x)).map(|(la, lo)| {
                let n = cache.entry(((la * 20.0).round() as i64, (lo * 20.0).round() as i64)).or_insert_with(|| place_name(la, lo));
                n.split(',').next().unwrap_or("").to_string()
            });
            let first = taken(0);
            let nm = name(place.as_deref(), &first, &taken(run.len() - 1));
            let mut pics: Vec<_> = run.iter().map(|(_, x)| x.clone()).collect();
            pics.reverse();
            out.push((first, nm, pics));
        }
        out.reverse();
        (out, waiting)
    })
    .await
    .unwrap_or_default();
    let (evs, waiting) = got;
    if let Some(id) = which {
        let Some(e) = evs.iter().find(|e| e.0 == id) else { return error(404, "no such event") };
        let mut photos: Vec<Value> = e.2.iter().map(|(id, r, g)| photo_json(r, id, *g)).collect();
        mark_stars(&mut photos, &stars);
        return send_json(&json!({"event": e.1, "id": e.0, "name": e.1, "photos": photos, "waiting": waiting}), 200);
    }
    let out: Vec<Value> = evs.iter().map(|(id, n, rows)| json!({"id": id, "name": n, "count": rows.len(), "cover": cover(rows)})).collect();
    send_json(&json!({"events": out, "waiting": waiting}), 200)
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: i64 = 3600;
    fn at(h: i64) -> (i64, Option<(f64, f64)>) {
        (h * H, None)
    }

    #[test]
    fn time_gap_splits_and_small_groups_drop() {
        let mut v: Vec<_> = (0..5).map(at).collect(); // 0..4 h: one run
        v.extend((20..25).map(at)); // 16 h gap, second run
        v.extend((40..43).map(at)); // only 3: dropped
        assert_eq!(group(&v, GAP_SECS, FAR_KM, 5), vec![0..5, 5..10]);
        // just under 6 h keeps it together, exactly 6 h splits
        let w: Vec<_> = [0, 5, 6, 7, 8, 9].iter().map(|h| at(*h)).collect();
        assert_eq!(group(&w, GAP_SECS, FAR_KM, 5), vec![0..6]);
        let w: Vec<_> = [0, 1, 2, 3, 4, 10, 11].iter().map(|h| at(*h)).collect();
        assert_eq!(group(&w, GAP_SECS, FAR_KM, 5), vec![0..5]);
    }

    #[test]
    fn distance_splits_only_with_positions() {
        let whitby = Some((54.49, -0.61));
        let london = Some((51.51, -0.12));
        let mut v: Vec<(i64, Option<(f64, f64)>)> = (0..5).map(|h| (h * H, whitby)).collect();
        v.extend((5..10).map(|h| (h * H, london)));
        assert_eq!(group(&v, GAP_SECS, FAR_KM, 5), vec![0..5, 5..10]);
        // pictures with no position never split by distance, and a nearby one doesn't either
        v[2].1 = None;
        v[7].1 = Some((51.52, -0.13));
        assert_eq!(group(&v, GAP_SECS, FAR_KM, 5), vec![0..5, 5..10]);
        let near: Vec<_> = (0..6).map(|h| (h * H, if h % 2 == 0 { whitby } else { None })).collect();
        assert_eq!(group(&near, GAP_SECS, FAR_KM, 5), vec![0..6]);
    }

    #[test]
    fn names() {
        assert_eq!(name(Some("Whitby"), "2026-08-03T10:00:00", "2026-08-05T18:00:00"), "Whitby · 3–5 Aug 2026");
        assert_eq!(name(Some("Whitby"), "2026-08-03T10:00:00", "2026-08-03T18:00:00"), "Whitby · 3 Aug 2026");
        assert_eq!(name(None, "2026-08-03T10:00:00", "2026-08-03T18:00:00"), "Monday 3 Aug 2026");
        assert_eq!(name(None, "2026-07-30T10:00:00", "2026-08-02T18:00:00"), "Thursday 30 Jul – 2 Aug 2026");
        assert_eq!(name(Some("X"), "2025-12-30T10:00:00", "2026-01-02T18:00:00"), "X · 30 Dec 2025 – 2 Jan 2026");
        assert_eq!(parse("1970-01-02T00:00:10"), Some((1, 10)));
    }
}
