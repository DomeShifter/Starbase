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
    assert_eq!((a.group_cut, a.suggest, a.auto), (0.75, 0.45, 0.55));
    let s = thresholds("sface");
    assert_eq!((s.group_cut, s.suggest, s.auto), (0.67, 0.54, 0.64));
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
    let p = faces::name(&ix, g, "ada", "Ann").unwrap();
    assert_eq!(faces::people(&ix, "ada").unwrap()[&p], "Ann");
    // Names are per account: bob doesn't see ada's name, and his own doesn't change hers.
    assert!(faces::people(&ix, "bob").unwrap().is_empty());
    assert_eq!(faces::name(&ix, Who::Person(p), "bob", "Annie").unwrap(), p);
    assert_eq!(faces::people(&ix, "ada").unwrap()[&p], "Ann");
    assert_eq!(faces::people(&ix, "bob").unwrap()[&p], "Annie");
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

/// A unit vector `a`·e0 + `b`·e1 + `c`·e(2+k): faces of a "new person" e1 that look a bit like e0.
fn mix(a: f32, b: f32, k: usize, c: f32) -> Vec<f32> {
    let mut v = [0f32; 16];
    v[0] = a;
    v[1] = b;
    v[2 + k] += c;
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter().map(|x| x / n).collect()
}

/// James 2026-11-26: new scans only ever added faces to existing people. Cause: faces matched
/// automatically were used as the person's faces too, so one borderline match of a new person
/// pulled every later face of them in. Now a new person's faces, scanned in separate passes,
/// form a new group, like on a first scan.
#[test]
fn a_new_person_in_a_later_scan_becomes_a_new_group() {
    let t = thresholds("auraface");
    let lock = index();
    {
        let ix = lock.lock().unwrap();
        // First scan: Ann, named.
        faces::keep(&ix, "a1", "auraface", &[found(near(0, 0, 0.1))]).unwrap();
        faces::keep(&ix, "a2", "auraface", &[found(near(0, 1, 0.1))]).unwrap();
    }
    faces::regroup(&lock, "auraface").unwrap();
    let ann = {
        let ix = lock.lock().unwrap();
        let g = faces::faces(&ix, "auraface").unwrap().iter().find_map(Who::of).unwrap();
        faces::name(&ix, g, "ada", "Ann").unwrap()
    };
    // Later scan, one batch at a time: Bea's first face happens to look a bit like Ann.
    let b1 = mix(0.6, 0.8, 0, 0.0);
    assert!(similarity(&b1, &near(0, 0, 0.1)) >= t.auto, "the borderline match this test is about");
    lock.lock().map(|ix| faces::keep(&ix, "b1", "auraface", &[found(b1)]).unwrap()).unwrap();
    faces::regroup(&lock, "auraface").unwrap();
    for (k, key) in ["b2", "b3"].iter().enumerate() {
        lock.lock().map(|ix| faces::keep(&ix, key, "auraface", &[found(mix(0.15, 1.0, k + 1, 0.1))]).unwrap()).unwrap();
        faces::regroup(&lock, "auraface").unwrap();
    }
    let ix = lock.lock().unwrap();
    let all = faces::faces(&ix, "auraface").unwrap();
    let bea: Vec<_> = all.iter().filter(|f| f.thumb.starts_with('b')).collect();
    assert!(bea.iter().all(|f| f.person.is_none()), "none of Bea's faces is Ann: {all:?}");
    let g = bea[0].grp;
    assert!(g.is_some() && bea.iter().all(|f| f.grp == g), "Bea is one new group: {all:?}");
    assert_eq!(bea.iter().find(|f| f.thumb == "b1").unwrap().suggest, Some(ann), "Ann is only offered");
    assert_eq!(all.iter().filter(|f| f.person == Some(ann)).count(), 2);
}

/// New faces of a known person still join them without asking.
#[test]
fn new_faces_of_a_known_person_still_join() {
    let lock = index();
    lock.lock().map(|ix| faces::keep(&ix, "a1", "auraface", &[found(near(0, 0, 0.1)), found(near(0, 1, 0.1))]).unwrap()).unwrap();
    faces::regroup(&lock, "auraface").unwrap();
    let ann = {
        let ix = lock.lock().unwrap();
        let g = faces::faces(&ix, "auraface").unwrap().iter().find_map(Who::of).unwrap();
        faces::name(&ix, g, "ada", "Ann").unwrap()
    };
    lock.lock().map(|ix| faces::keep(&ix, "a2", "auraface", &[found(near(0, 2, 0.2)), found(near(0, 3, 0.2))]).unwrap()).unwrap();
    faces::regroup(&lock, "auraface").unwrap();
    // Looked at again in later passes: they stay.
    faces::regroup(&lock, "auraface").unwrap();
    let ix = lock.lock().unwrap();
    assert_eq!(faces::faces(&ix, "auraface").unwrap().iter().filter(|f| f.person == Some(ann)).count(), 4);
}

/// Face pictures are cut from the big thumbnail: a face 10% of a 1024 px picture is ~154 px of
/// it (from the 256 one it was ~38 px), scaled to the 256 px People asks for.
#[test]
fn face_crop_is_square_and_sharp() {
    let mut img = image::RgbImage::new(1024, 768);
    for (x, y, p) in img.enumerate_pixels_mut() {
        *p = image::Rgb([(x / 4) as u8, (y / 3) as u8, 0]);
    }
    let c = faces::face_crop(&img, [0.45, 0.4, 0.1, 0.1], 256);
    assert_eq!(c.dimensions(), (256, 256));
    // The face's centre stays in the middle.
    let mid = c.get_pixel(128, 128);
    assert!((mid[0] as i32 - 128).abs() <= 2 && (mid[1] as i32 - 115).abs() <= 2, "{mid:?}");
    // Near an edge it moves inside the picture; bigger than the picture, it's the whole height.
    assert_eq!(faces::face_crop(&img, [0.95, 0.0, 0.05, 0.05], 256).dimensions(), (256, 256));
    assert_eq!(faces::face_crop(&img, [0.0, 0.0, 1.0, 1.0], 64).dimensions(), (64, 64));
    assert_eq!(faces::face_crop(&image::RgbImage::new(0, 0), [0.1, 0.1, 0.1, 0.1], 32).dimensions(), (32, 32));
}

/// Ann, named from two faces (near 0), in a fresh index.
fn with_ann() -> (std::sync::Mutex<Index>, i64) {
    let lock = index();
    lock.lock().map(|ix| faces::keep(&ix, "a1", "auraface", &[found(near(0, 0, 0.1)), found(near(0, 1, 0.1))]).unwrap()).unwrap();
    faces::regroup(&lock, "auraface").unwrap();
    let ann = {
        let ix = lock.lock().unwrap();
        let g = faces::faces(&ix, "auraface").unwrap().iter().find_map(Who::of).unwrap();
        faces::name(&ix, g, "ada", "Ann").unwrap()
    };
    (lock, ann)
}

fn by_thumb(lock: &std::sync::Mutex<Index>, thumb: &str) -> faces::Face {
    let ix = lock.lock().unwrap();
    faces::faces(&ix, "auraface").unwrap().into_iter().find(|f| f.thumb == thumb).unwrap()
}

/// James 2026-11-27: a face removed from a person comes back as its own unnamed group (it can be
/// named or merged) and is never put back with that person, however like them it is.
#[test]
fn a_removed_face_is_a_new_face_and_never_rejoins() {
    let (lock, ann) = with_ann();
    lock.lock().map(|ix| faces::keep(&ix, "a2", "auraface", &[found(near(0, 2, 0.1))]).unwrap()).unwrap();
    faces::regroup(&lock, "auraface").unwrap();
    let f = by_thumb(&lock, "a2");
    assert_eq!(f.person, Some(ann), "a sure match joins on its own");
    assert!(lock.lock().map(|ix| faces::remove(&ix, Who::Person(ann), f.id).unwrap()).unwrap());
    for _ in 0..2 {
        let g = by_thumb(&lock, "a2");
        assert_eq!((g.person, g.grp), (None, Some(f.id)), "its own unnamed group");
        faces::regroup(&lock, "auraface").unwrap();
    }
    let ix = lock.lock().unwrap();
    // Nameable: it becomes a person of its own.
    let p = faces::name(&ix, Who::Group(f.id), "ada", "Bea").unwrap();
    assert_ne!(p, ann);
    drop(ix);
    faces::regroup(&lock, "auraface").unwrap();
    assert_eq!(by_thumb(&lock, "a2").person, Some(p));
}

/// A face only somewhat like Ann (between "maybe" and "sure") isn't joined: it is asked about.
#[test]
fn a_borderline_match_is_asked_not_joined() {
    let t = thresholds("auraface");
    let (lock, ann) = with_ann();
    let e = mix(0.5, 0.866, 3, 0.0);
    let sim = similarity(&e, &near(0, 0, 0.1)).max(similarity(&e, &near(0, 1, 0.1)));
    assert!(sim >= t.suggest && sim < t.auto, "borderline: {sim}");
    lock.lock().map(|ix| faces::keep(&ix, "b1", "auraface", &[found(e)]).unwrap()).unwrap();
    faces::regroup(&lock, "auraface").unwrap();
    let f = by_thumb(&lock, "b1");
    assert_eq!((f.person, f.suggest), (None, Some(ann)));
    // Yes: it joins Ann, and stays there.
    assert!(lock.lock().map(|ix| faces::answer(&ix, f.id, ann, true).unwrap()).unwrap());
    faces::regroup(&lock, "auraface").unwrap();
    assert_eq!(by_thumb(&lock, "b1").person, Some(ann));
}

/// "No" is kept: never asked again, never joined, even after the person gets more faces.
#[test]
fn no_is_remembered() {
    let (lock, ann) = with_ann();
    lock.lock().map(|ix| faces::keep(&ix, "b1", "auraface", &[found(mix(0.5, 0.866, 3, 0.0))]).unwrap()).unwrap();
    faces::regroup(&lock, "auraface").unwrap();
    let f = by_thumb(&lock, "b1");
    assert_eq!(f.suggest, Some(ann));
    assert!(lock.lock().map(|ix| faces::answer(&ix, f.id, ann, false).unwrap()).unwrap());
    // Ann gets a face very like this one (it lands in b1's group; "Yes" to it): b1 is still
    // never joined or asked about.
    lock.lock().map(|ix| faces::keep(&ix, "a3", "auraface", &[found(mix(0.5, 0.866, 4, 0.05))]).unwrap()).unwrap();
    faces::regroup(&lock, "auraface").unwrap();
    let a3 = by_thumb(&lock, "a3");
    assert_eq!(a3.suggest, Some(ann));
    assert!(lock.lock().map(|ix| faces::answer(&ix, a3.id, ann, true).unwrap()).unwrap());
    assert_eq!(by_thumb(&lock, "a3").person, Some(ann));
    faces::regroup(&lock, "auraface").unwrap();
    let g = by_thumb(&lock, "b1");
    assert_eq!((g.person, g.suggest), (None, None), "{g:?}");
    assert!(!lock.lock().map(|ix| faces::answer(&ix, 999_999, ann, true).unwrap()).unwrap(), "no such face");
}

fn found_at(e: Vec<f32>, x: f32) -> Found {
    Found { x, y: 0.1, w: 0.2, h: 0.2, score: 0.95, px: 80.0, emb: e }
}

/// James 2026-11-27: ignoring a group takes it out of People and grouping; undoing brings the
/// same group back. Nothing is deleted.
#[test]
fn ignore_a_group_and_undo() {
    let lock = index();
    lock.lock().map(|ix| faces::keep(&ix, "s1", "auraface", &[found(near(2, 0, 0.1)), found(near(2, 1, 0.1))]).unwrap()).unwrap();
    faces::regroup(&lock, "auraface").unwrap();
    let g = by_thumb(&lock, "s1").grp.expect("a group");
    let ids: Vec<i64> =
        lock.lock().map(|ix| faces::faces(&ix, "auraface").unwrap().iter().filter(|f| f.grp == Some(g)).map(|f| f.id).collect()).unwrap();
    assert_eq!(ids.len(), 2);
    assert_eq!(lock.lock().map(|ix| faces::mark(&ix, &ids, faces::IGNORED).unwrap()).unwrap(), 2);
    faces::regroup(&lock, "auraface").unwrap();
    {
        let ix = lock.lock().unwrap();
        assert!(faces::faces(&ix, "auraface").unwrap().is_empty(), "out of People and grouping");
        let m = faces::marked(&ix, "auraface").unwrap();
        assert_eq!(m.len(), 2);
        assert!(m.iter().all(|f| f.state == faces::IGNORED && f.grp == Some(g)));
        // Not on the "not a face" list: undoing that does nothing.
        assert_eq!(faces::unmark(&ix, &ids, faces::NOT_FACE, false).unwrap(), 0);
        assert_eq!(faces::unmark(&ix, &ids, faces::IGNORED, false).unwrap(), 2);
    }
    faces::regroup(&lock, "auraface").unwrap();
    let ix = lock.lock().unwrap();
    let all = faces::faces(&ix, "auraface").unwrap();
    assert!(all.len() == 2 && all.iter().all(|f| f.grp == Some(g)), "the same group again: {all:?}");
}

/// An ignored face of a person: never that person again, never asked about; "Assign to…" gives
/// it to someone; "Group again" lets grouping decide afresh.
#[test]
fn an_ignored_face_leaves_its_person_and_can_be_assigned_or_grouped_again() {
    let (lock, ann) = with_ann();
    let f = by_thumb(&lock, "a1");
    assert_eq!(f.person, Some(ann));
    lock.lock().map(|ix| faces::mark(&ix, &[f.id], faces::IGNORED).unwrap()).unwrap();
    faces::regroup(&lock, "auraface").unwrap();
    {
        let ix = lock.lock().unwrap();
        assert!(faces::faces(&ix, "auraface").unwrap().iter().all(|g| g.id != f.id));
        assert!(!faces::answer(&ix, f.id, ann, true).unwrap(), "no questions about an ignored face");
        // Group again: back without a group; regrouping never puts it with Ann (taken from her).
        assert_eq!(faces::unmark(&ix, &[f.id], faces::IGNORED, true).unwrap(), 1);
    }
    faces::regroup(&lock, "auraface").unwrap();
    let g = lock.lock().unwrap();
    let back = faces::face(&g, f.id).unwrap().unwrap();
    assert_eq!((back.person, back.state), (None, 0));
    // Ignore again, then assign it to Ann after all.
    faces::mark(&g, &[f.id], faces::IGNORED).unwrap();
    assert_eq!(faces::assign(&g, &[f.id], ann).unwrap(), 1);
    let back = faces::face(&g, f.id).unwrap().unwrap();
    assert_eq!((back.person, back.state), (Some(ann), 0));
    assert_eq!(faces::assign(&g, &[f.id], ann).unwrap(), 0, "only marked faces are assigned here");
}

/// "Not a face" survives the picture being looked at again (same box, a little off); a face
/// somewhere else in the picture is new. "It is a face" undoes it.
#[test]
fn not_a_face_is_not_found_again_on_a_rescan() {
    let lock = index();
    lock.lock().map(|ix| faces::keep(&ix, "d1", "auraface", &[found_at(near(3, 0, 0.1), 0.1), found_at(near(4, 0, 0.1), 0.6)]).unwrap()).unwrap();
    let doll = by_thumb(&lock, "d1")
        .id
        .min(lock.lock().map(|ix| faces::faces(&ix, "auraface").unwrap().iter().find(|f| f.bx[0] < 0.3).unwrap().id).unwrap());
    lock.lock().map(|ix| faces::mark(&ix, &[doll], faces::NOT_FACE).unwrap()).unwrap();
    // Looked at again: the doll's box a little moved, the other face still there.
    lock.lock().map(|ix| faces::keep(&ix, "d1", "auraface", &[found_at(near(3, 1, 0.1), 0.12), found_at(near(4, 1, 0.1), 0.6)]).unwrap()).unwrap();
    faces::regroup(&lock, "auraface").unwrap();
    let ix = lock.lock().unwrap();
    let shown = faces::faces(&ix, "auraface").unwrap();
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert!((shown[0].bx[0] - 0.6).abs() < 1e-6);
    let m = faces::marked(&ix, "auraface").unwrap();
    assert_eq!((m.len(), m[0].state), (1, faces::NOT_FACE));
    assert!(faces::overlap([0.1, 0.1, 0.2, 0.2], [0.6, 0.1, 0.2, 0.2]) < faces::SAME_BOX);
    assert_eq!(faces::unmark(&ix, &[m[0].id], faces::NOT_FACE, false).unwrap(), 1);
    assert_eq!(faces::faces(&ix, "auraface").unwrap().len(), 2, "It is a face: back");
}

fn at(x: f32, y: f32, e: Vec<f32>) -> Found {
    Found { x, y, ..found(e) }
}

#[test]
fn recheck_adds_only_missed_faces() {
    let lock = index();
    {
        let ix = lock.lock().unwrap();
        // Looked at by the old finder: Ann's face and a doll; her friend next to her was missed.
        faces::keep(&ix, "aa", "auraface", &[at(0.1, 0.1, near(0, 0, 0.1)), at(0.6, 0.6, near(2, 0, 0.1))]).unwrap();
        faces::keep(&ix, "bb", "auraface", &[at(0.1, 0.1, near(0, 1, 0.1))]).unwrap();
        ix.conn().execute("UPDATE faces_done SET detect = 1", []).unwrap();
        assert_eq!(faces::recheck_progress(&ix, "auraface"), (0, 2));
    }
    faces::regroup(&lock, "auraface").unwrap();
    let ix = lock.lock().unwrap();
    let before = faces::faces(&ix, "auraface").unwrap();
    let ann_face = before.iter().find(|f| f.thumb == "aa" && f.bx[0] < 0.5).unwrap().clone();
    let doll = before.iter().find(|f| f.bx[0] > 0.5).unwrap().id;
    let ann = faces::name(&ix, Who::of(&ann_face).unwrap(), "ada", "Ann").unwrap();
    assert_eq!(faces::mark(&ix, &[doll], faces::NOT_FACE).unwrap(), 1);
    ix.conn().execute("INSERT INTO face_not(face, person) VALUES(?1, 99)", [ann_face.id]).unwrap();
    let todo = faces::recheck_todo(&ix, "auraface", 10).unwrap();
    assert_eq!(todo.len(), 2);
    assert!(faces::todo(&ix, "auraface", 10).unwrap().is_empty(), "the re-check is not new work");
    // The new finder finds Ann (moved a little), the doll, and the friend right next to Ann
    // (overlap 0.33: a different face).
    let again = [at(0.11, 0.1, near(0, 0, 0.1)), at(0.6, 0.61, near(2, 0, 0.1)), at(0.2, 0.1, near(1, 0, 0.1))];
    assert_eq!(faces::add_new(&ix, "aa", "auraface", &again).unwrap(), 1);
    assert_eq!(faces::add_new(&ix, "bb", "auraface", &[at(0.1, 0.1, near(0, 1, 0.1)), at(0.2, 0.1, near(1, 1, 0.1))]).unwrap(), 1);
    assert_eq!(faces::recheck_progress(&ix, "auraface"), (2, 2));
    assert!(faces::recheck_todo(&ix, "auraface", 10).unwrap().is_empty());
    // Nothing that was there changed: Ann's face, its name, the doll's mark, "not this person".
    let f = faces::face(&ix, ann_face.id).unwrap().unwrap();
    assert_eq!((f.person, f.bx), (Some(ann), ann_face.bx));
    assert_eq!(faces::people(&ix, "ada").unwrap()[&ann], "Ann");
    assert_eq!(faces::face(&ix, doll).unwrap().unwrap().state, faces::NOT_FACE);
    let nots: i64 = ix.conn().query_row("SELECT COUNT(*) FROM face_not WHERE face = ?1", [ann_face.id], |r| r.get(0)).unwrap();
    assert_eq!(nots, 1);
    let all = faces::faces(&ix, "auraface").unwrap();
    assert_eq!(all.len(), 4, "Ann and her friend in both (the doll is out of the list)");
    let friend = all.iter().find(|f| f.thumb == "aa" && f.id != ann_face.id).unwrap();
    assert_eq!(friend.person, None, "a new face is unnamed until grouped");
    // A second run adds nothing, even when asked directly.
    assert_eq!(faces::add_new(&ix, "aa", "auraface", &again).unwrap(), 0);
    drop(ix);
    // The friend goes through normal grouping: one new group, not Ann.
    faces::regroup(&lock, "auraface").unwrap();
    let ix = lock.lock().unwrap();
    let friends: Vec<_> = faces::faces(&ix, "auraface").unwrap().into_iter().filter(|f| f.bx[0] > 0.15).collect();
    assert_eq!(friends.len(), 2);
    assert!(friends.iter().all(|f| f.person.is_none() && f.grp.is_some() && f.grp == friends[0].grp), "{friends:?}");
}

#[test]
fn new_pictures_are_not_rechecked() {
    let lock = index();
    let ix = lock.lock().unwrap();
    faces::keep(&ix, "cc", "auraface", &[found(near(0, 0, 0.1))]).unwrap();
    assert!(faces::recheck_todo(&ix, "auraface", 10).unwrap().is_empty());
    assert_eq!(faces::recheck_progress(&ix, "auraface"), (1, 1));
}
