//! Faces: grouping, thresholds, the people changes, and the pure parts of the face worker.
//! None of these need the models (they run in CI); a real model run needs `--features ml` and
//! the model files, and is checked by hand (see ml.rs).

use std::collections::{HashMap, HashSet};

use super::faces::{self, Found, Snapshot, Who, average_linkage, plan, similarity, thresholds};
use super::index::Index;
use super::ml::{self, Det, TEMPLATE, YuStride, align, nms, similarity_transform, yunet_decode};

/// A unit vector near `base` (a person), nudged by `k` in another direction.
fn near(base: usize, k: usize, amount: f32) -> Vec<f32> {
    let mut v = [0f32; 16];
    v[base] = 1.0;
    v[8 + (k % 8)] += amount;
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter().map(|x| x / n).collect()
}

#[test]
fn thresholds_are_the_measured_ones() {
    let a = thresholds("auraface");
    assert_eq!((a.group_cut, a.suggest, a.auto), (0.75, 0.35, 0.45));
    let s = thresholds("sface");
    assert_eq!((s.group_cut, s.suggest, s.auto), (0.67, 0.43, 0.54));
    assert!(a.suggest < a.auto && s.suggest < s.auto);
}

#[test]
fn similarity_basics() {
    assert!((similarity(&[1.0, 0.0], &[2.0, 0.0]) - 1.0).abs() < 1e-6);
    assert!(similarity(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
    assert_eq!(similarity(&[0.0, 0.0], &[1.0, 0.0]), 0.0);
}

#[test]
fn average_linkage_separates_people() {
    // Three people, four faces each.
    let mut e = Vec::new();
    for p in 0..3 {
        for k in 0..4 {
            e.push(near(p, k, 0.3));
        }
    }
    let l = average_linkage(&e, 0.75);
    for p in 0..3 {
        let g: HashSet<usize> = (0..4).map(|k| l[p * 4 + k]).collect();
        assert_eq!(g.len(), 1, "one person, one group: {l:?}");
    }
    let all: HashSet<usize> = l.iter().copied().collect();
    assert_eq!(all.len(), 3, "three groups: {l:?}");
    // A cut of 0 keeps every (different) face alone; a cut of 2 joins everything.
    assert_eq!(average_linkage(&e, 0.0).iter().collect::<HashSet<_>>().len(), 12);
    assert_eq!(average_linkage(&e, 2.0).iter().collect::<HashSet<_>>().len(), 1);
    assert!(average_linkage(&[], 0.75).is_empty());
    assert_eq!(average_linkage(&[vec![1.0, 0.0]], 0.75), vec![0]);
}

#[test]
fn average_linkage_does_not_chain() {
    // A line of faces each close to the next, ends far apart: single linkage (and DBSCAN) chain
    // them into one group; average linkage must not join the two ends.
    let e: Vec<Vec<f32>> = (0..10)
        .map(|i| {
            let a = i as f32 * 0.25;
            vec![a.cos(), a.sin()]
        })
        .collect();
    let l = average_linkage(&e, 0.3);
    assert_ne!(l[0], l[9], "{l:?}");
}

#[test]
fn plan_matches_people_then_groups() {
    let t = thresholds("auraface");
    let mut people = HashMap::new();
    people.insert(7, vec![near(0, 0, 0.0)]);
    let snap = Snapshot {
        loose: vec![
            (1, near(0, 1, 0.2), true), // very like person 7: joins
            (2, near(0, 2, 1.6), true), // somewhat like person 7: offered (sim ~0.53 > auto? see below)
            (3, near(1, 0, 0.1), true), // a new person, two faces: a group
            (4, near(1, 1, 0.1), true),
            (5, near(2, 0, 0.0), true),  // alone: no group
            (6, near(1, 2, 0.1), false), // like 3/4 but not good enough to group
        ],
        people,
        not: HashMap::new(),
    };
    let s2 = similarity(&snap.loose[1].1, &snap.people[&7][0]);
    let p = plan(&snap, t);
    let row = |id: i64| *p.iter().find(|r| r.0 == id).unwrap();
    assert_eq!(row(1).1, Some(7));
    if s2 >= t.auto {
        assert_eq!(row(2).1, Some(7));
    } else if s2 >= t.suggest {
        assert_eq!((row(2).1, row(2).2), (None, Some(7)));
    }
    assert_eq!(row(3).3, Some(3));
    assert_eq!(row(4).3, Some(3), "a group is named by its smallest face id");
    assert_eq!(row(5).3, None, "groups of one stay loose");
    assert_eq!(row(6).3, None, "weak faces never group");
    // A face taken away from a person is never matched to them again.
    let mut snap = snap;
    snap.not.insert(1, HashSet::from([7]));
    assert_eq!(plan(&snap, t).iter().find(|r| r.0 == 1).unwrap().1, None);
}

fn found(e: Vec<f32>) -> Found {
    Found { x: 0.1, y: 0.1, w: 0.2, h: 0.2, score: 0.95, px: 80.0, emb: e }
}

fn index() -> std::sync::Mutex<Index> {
    let dir = std::env::temp_dir().join(format!("starbase-faces-{}", crate::store::random_hex(6)));
    std::fs::create_dir_all(&dir).unwrap();
    std::sync::Mutex::new(Index::open(&dir.join("index.sqlite")).unwrap())
}

#[test]
fn name_merge_remove_and_regroup() {
    let lock = index();
    {
        let ix = lock.lock().unwrap();
        faces::keep(&ix, "aa", "auraface", &[found(near(0, 0, 0.1)), found(near(1, 0, 0.1))]).unwrap();
        faces::keep(&ix, "bb", "auraface", &[found(near(0, 1, 0.1)), found(near(1, 1, 0.1))]).unwrap();
        // Kept again: replaced, not doubled.
        faces::keep(&ix, "bb", "auraface", &[found(near(0, 1, 0.1)), found(near(1, 1, 0.1))]).unwrap();
        assert_eq!(faces::faces(&ix, "auraface").unwrap().len(), 4);
    }
    faces::regroup(&lock, "auraface").unwrap();
    let ix = lock.lock().unwrap();
    let all = faces::faces(&ix, "auraface").unwrap();
    let groups: HashSet<Who> = all.iter().filter_map(Who::of).collect();
    assert_eq!(groups.len(), 2, "{all:?}");
    // Name one group: it becomes a person.
    let g = *groups.iter().next().unwrap();
    let p = faces::name(&ix, g, "Ann").unwrap();
    assert_eq!(faces::people(&ix).unwrap()[&p], "Ann");
    let ann: Vec<_> = faces::faces(&ix, "auraface").unwrap().into_iter().filter(|f| f.person == Some(p)).collect();
    assert_eq!(ann.len(), 2);
    // Merge the other group into Ann, then take one wrong face out.
    let other = *groups.iter().find(|w| **w != g).unwrap();
    assert_eq!(faces::merge(&ix, other, Who::Person(p)).unwrap(), p);
    assert_eq!(faces::faces(&ix, "auraface").unwrap().iter().filter(|f| f.person == Some(p)).count(), 4);
    let wrong = faces::faces(&ix, "auraface").unwrap().into_iter().find(|f| f.person == Some(p)).unwrap().id;
    assert!(faces::remove(&ix, Who::Person(p), wrong).unwrap());
    assert!(!faces::remove(&ix, Who::Person(p), wrong).unwrap(), "already out");
    drop(ix);
    // Regrouping never puts it back with Ann.
    faces::regroup(&lock, "auraface").unwrap();
    let ix = lock.lock().unwrap();
    let f = faces::faces(&ix, "auraface").unwrap().into_iter().find(|f| f.id == wrong).unwrap();
    assert_eq!(f.person, None);
    // Progress: two thumbnails looked at.
    assert_eq!(faces::progress(&ix, "auraface").2, 4);
    assert!(faces::todo(&ix, "auraface", 10).unwrap().is_empty());
}

#[test]
fn who_ids_round_trip() {
    assert_eq!(Who::parse("p12"), Some(Who::Person(12)));
    assert_eq!(Who::parse("g3").map(Who::key).as_deref(), Some("g3"));
    assert_eq!(Who::parse("x1"), None);
    assert_eq!(Who::parse("p"), None);
    assert_eq!(Who::parse(""), None);
}

#[test]
fn similarity_transform_recovers_a_known_one() {
    let id = similarity_transform(&TEMPLATE, &TEMPLATE);
    assert!((id[0] - 1.0).abs() < 1e-4 && id[1].abs() < 1e-4 && id[2].abs() < 1e-3 && id[3].abs() < 1e-3, "{id:?}");
    // Landmarks at twice the size and shifted: the transform halves and shifts back.
    let src = TEMPLATE.map(|[x, y]| [2.0 * x + 50.0, 2.0 * y + 20.0]);
    let [a, b, tx, ty] = similarity_transform(&src, &TEMPLATE);
    assert!((a - 0.5).abs() < 1e-4 && b.abs() < 1e-4 && (tx + 25.0).abs() < 1e-2 && (ty + 10.0).abs() < 1e-2);
}

#[test]
fn align_takes_the_face() {
    // A picture whose left half is red: a face placed at the template in its left half comes out red.
    let img = image::RgbImage::from_fn(400, 200, |x, _| if x < 200 { image::Rgb([255, 0, 0]) } else { image::Rgb([0, 0, 255]) });
    let kps = TEMPLATE.map(|[x, y]| [x + 40.0, y + 40.0]);
    let face = align(&img, &kps);
    assert_eq!(face.dimensions(), (112, 112));
    assert_eq!(face.get_pixel(56, 56).0, [255, 0, 0]);
}

#[test]
fn yunet_decoding_and_nms() {
    // One 32×32 input, stride 32: one cell with a face, score sqrt(0.9 × 0.9) = 0.9.
    let cls = [0.9f32];
    let obj = [0.9f32];
    let bbox = [0.5f32, 0.5, 0.0, 0.0]; // centre of the cell, 32×32
    let kps = [0.2f32, 0.3, 0.8, 0.3, 0.5, 0.5, 0.3, 0.7, 0.7, 0.7];
    let s = [YuStride { stride: 32, cls: &cls, obj: &obj, bbox: &bbox, kps: &kps }];
    let d = yunet_decode(&s, 32, 32, 0.7);
    assert_eq!(d.len(), 1);
    assert!((d[0].score - 0.9).abs() < 1e-5);
    assert_eq!((d[0].x, d[0].y, d[0].w, d[0].h), (0.0, 0.0, 32.0, 32.0));
    assert!((d[0].kps[1][0] - 25.6).abs() < 1e-4);
    // Below the score: dropped; short outputs: no panic.
    assert!(yunet_decode(&s, 32, 32, 0.95).is_empty());
    let short = [YuStride { stride: 8, cls: &cls, obj: &obj, bbox: &bbox, kps: &kps }];
    assert_eq!(yunet_decode(&short, 32, 32, 0.7).len(), 1);
    let a = Det { x: 0.0, y: 0.0, w: 10.0, h: 10.0, score: 0.9, kps: [[0.0; 2]; 5] };
    let b = Det { x: 1.0, y: 1.0, w: 10.0, h: 10.0, score: 0.8, kps: [[0.0; 2]; 5] };
    let c = Det { x: 50.0, y: 50.0, w: 10.0, h: 10.0, score: 0.7, kps: [[0.0; 2]; 5] };
    let kept = nms(vec![b, c.clone(), a.clone()], 0.3);
    assert_eq!(kept, vec![a, c]);
}

#[test]
fn inputs_have_the_right_shape() {
    let img = image::RgbImage::from_pixel(40, 33, image::Rgb([10, 20, 30]));
    let (scale, v) = ml::yunet_input(&img);
    let n = ml::YUNET_SIZE;
    assert_eq!(v.len(), 3 * n * n, "always the model's fixed square");
    assert!((scale - n as f32 / 40.0).abs() < 1e-6);
    assert_eq!((v[0], v[n * n], v[2 * n * n]), (30.0, 20.0, 10.0), "BGR");
    assert_eq!(v[n * n - 1], 0.0, "padding below the picture is black");
    let face = image::RgbImage::from_pixel(112, 112, image::Rgb([255, 0, 127]));
    let a = ml::embed_input(&face, "auraface");
    assert_eq!(a.len(), 3 * 112 * 112);
    assert!((a[0] - 1.0).abs() < 1e-6 && (a[112 * 112] + 1.0).abs() < 1e-6);
    assert_eq!(ml::embed_input(&face, "sface")[0], 255.0);
}

#[test]
fn registry_names_files_and_licences() {
    for n in ["yunet", "auraface", "sface"] {
        let m = ml::model_file(n).unwrap();
        assert!(m.file.ends_with(".onnx") && !m.licence.is_empty());
        assert!(!m.licence.contains("NC"), "only permissive models");
    }
    assert!(ml::model_file("buffalo_l").is_none());
    // The button's files are pinned: an exact https URL, a SHA-256 and a size.
    for n in ["yunet", "auraface"] {
        let g = ml::model_file(n).unwrap().get.as_ref().unwrap();
        assert!(g.url.starts_with("https://") && g.url.ends_with(ml::model_file(n).unwrap().file), "{}", g.url);
        assert!(g.sha256.len() == 64 && g.sha256.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        assert!(g.size > 100_000);
    }
}
