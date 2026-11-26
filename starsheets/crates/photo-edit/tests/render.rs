//! Public-API tests: the model (JSON in and out, validation) and every render step on small
//! synthetic images whose expected pixels are known.

use image::{Rgba, RgbaImage};
use starbase_photo_edit::{Crop, Edits, Filter, FilterName, output_size, render};

/// A picture where every pixel is different (so moves are visible), opaque.
fn pattern(w: u32, h: u32) -> RgbaImage {
    RgbaImage::from_fn(w, h, |x, y| Rgba([(x * 7 + y * 3) as u8, (x * 13 + y * 29) as u8, (x * 5 + y * 17 + 40) as u8, 255]))
}

fn flat(w: u32, h: u32, c: [u8; 3]) -> RgbaImage {
    RgbaImage::from_pixel(w, h, Rgba([c[0], c[1], c[2], 255]))
}

fn px(img: &RgbaImage, x: u32, y: u32) -> [u8; 4] {
    img.get_pixel(x, y).0
}

fn parse(s: &str) -> Result<Edits, serde_json::Error> {
    serde_json::from_str(s)
}

// ---- model ----

#[test]
fn default_is_identity_and_empty_json_is_default() {
    assert!(Edits::default().is_identity());
    assert_eq!(parse("{}").unwrap(), Edits::default());
    assert_eq!(parse(r#"{"v":1,"aspect":null,"filter":null,"exposure":null}"#).unwrap(), Edits::default());
    // The aspect hint doesn't change pixels; a filter at amount 0 doesn't either.
    assert!(parse(r#"{"aspect":"4:3","filter":{"name":"bw","amount":0}}"#).unwrap().is_identity());
}

#[test]
fn serde_round_trip() {
    let e = Edits {
        rot90: 3,
        flip_h: true,
        angle: -12.3,
        crop: Crop { l: 0.1, t: 0.2, r: 0.9, b: 0.75 },
        aspect: Some(starbase_photo_edit::Aspect::R16x9),
        exposure: 0.7,
        contrast: 20,
        highlights: -40,
        shadows: 35,
        saturation: -10,
        vibrance: 50,
        temperature: 15,
        tint: -5,
        filter: Some(Filter { name: FilterName::Sepia, amount: 0.3 }),
        red_eye: vec![],
    };
    let s = serde_json::to_string(&e).unwrap();
    assert!(s.contains(r#""angle":-12.3"#) && s.contains(r#""aspect":"16:9""#) && s.contains(r#""v":1"#), "{s}");
    assert_eq!(parse(&s).unwrap(), e);
    let d = serde_json::to_string(&Edits::default()).unwrap();
    assert_eq!(parse(&d).unwrap(), Edits::default());
    // Tiny crops written and read back stay valid.
    let tiny = Edits { crop: Crop { l: 0.3, t: 0.6, r: 0.31, b: 0.61 }, ..Edits::default() };
    assert!(tiny.validate().is_ok());
    assert_eq!(parse(&serde_json::to_string(&tiny).unwrap()).unwrap(), tiny);
}

#[test]
fn validation_rejects_bad_input() {
    for bad in [
        r#"{"v":2}"#,
        r#"{"rot90":4}"#,
        r#"{"rot90":-1}"#,
        r#"{"rot90":99999999999999}"#,
        r#"{"angle":45.01}"#,
        r#"{"angle":1e300}"#,
        r#"{"angle":1e999}"#,
        r#"{"exposure":-5.5}"#,
        r#"{"exposure":"NaN"}"#,
        r#"{"exposure":NaN}"#,
        r#"{"contrast":101}"#,
        r#"{"tint":-101}"#,
        r#"{"saturation":12.5}"#,
        r#"{"vibrance":1000000000000000000000}"#,
        r#"{"crop":{"l":0.5,"r":0.4}}"#,
        r#"{"crop":{"l":0.5,"r":0.505}}"#,
        r#"{"crop":{"t":-0.1}}"#,
        r#"{"crop":{"b":1.5}}"#,
        r#"{"crop":{"l":0.2,"t":0.2,"r":0.2,"b":0.9}}"#,
        r#"{"filter":{"name":"bw","amount":1.5}}"#,
        r#"{"filter":{"name":"lomo","amount":0.5}}"#,
        r#"{"filter":{"amount":0.5}}"#,
        r#"{"aspect":"5:4"}"#,
        r#"{"flip_h":"yes"}"#,
        r#""x""#,
    ] {
        assert!(parse(bad).is_err(), "accepted {bad}");
    }
    for exposure in [f32::NAN, f32::INFINITY] {
        assert!(Edits { exposure, ..Edits::default() }.validate().is_err());
    }
    let e = Edits { rot90: 7, ..Edits::default() };
    assert!(e.validate().is_err());
    let e = Edits { crop: Crop { l: 0.5, t: 0.0, r: 0.2, b: 1.0 }, ..Edits::default() };
    assert!(e.validate().is_err());
    // Unknown fields are ignored (a newer writer may add some).
    assert_eq!(parse(r#"{"future":[1,2,3]}"#).unwrap(), Edits::default());
}

#[test]
fn bad_edits_made_in_code_never_panic() {
    let img = pattern(31, 17);
    let wild = Edits {
        rot90: 200,
        angle: f32::NAN,
        crop: Crop { l: f32::INFINITY, t: 0.9, r: -3.0, b: 0.1 },
        exposure: 1e30,
        contrast: i32::MAX,
        highlights: i32::MIN,
        shadows: 5000,
        saturation: -5000,
        vibrance: i32::MAX,
        temperature: i32::MIN,
        tint: 777,
        filter: Some(Filter { name: FilterName::Vivid, amount: f32::NAN }),
        ..Edits::default()
    };
    let out = render(&img, &wild);
    assert_eq!(out.dimensions(), output_size(31, 17, &wild));
    let one = RgbaImage::from_pixel(1, 1, Rgba([9, 9, 9, 255]));
    let e = Edits { angle: 45.0, crop: Crop { l: 0.0, t: 0.0, r: 0.01, b: 0.01 }, ..Edits::default() };
    assert_eq!(render(&one, &e).dimensions(), (1, 1));
    let empty = RgbaImage::new(0, 0);
    assert_eq!(render(&empty, &e).dimensions(), (0, 0));
}

// ---- rendering ----

#[test]
fn identity_is_byte_identical() {
    let img = pattern(64, 48);
    assert_eq!(render(&img, &Edits::default()), img);
    let hinted = parse(r#"{"aspect":"1:1","filter":{"name":"sepia","amount":0}}"#).unwrap();
    assert_eq!(render(&img, &hinted), img);
}

#[test]
fn untouched_steps_keep_bytes() {
    // Only geometry: pixels are copied exactly. Only a slider that does nothing to grey: grey stays.
    let img = pattern(40, 30);
    let e = Edits { rot90: 2, ..Edits::default() };
    assert_eq!(render(&render(&img, &e), &e), img);
    let grey = flat(8, 8, [128, 128, 128]);
    let e = Edits { saturation: 80, vibrance: -60, ..Edits::default() };
    assert_eq!(render(&grey, &e), grey);
}

#[test]
fn quarter_turns_and_flip() {
    let img = pattern(5, 3);
    let r1 = render(&img, &Edits { rot90: 1, ..Edits::default() });
    assert_eq!(r1.dimensions(), (3, 5));
    // Clockwise: the top-left corner goes to the top-right.
    assert_eq!(px(&r1, 2, 0), px(&img, 0, 0));
    assert_eq!(px(&r1, 0, 0), px(&img, 0, 2));
    let r3 = render(&img, &Edits { rot90: 3, ..Edits::default() });
    assert_eq!(px(&r3, 0, 4), px(&img, 0, 0));
    let f = render(&img, &Edits { flip_h: true, ..Edits::default() });
    assert_eq!(px(&f, 4, 1), px(&img, 0, 1));
    // Turn first, then flip: top-left ends up top-left.
    let rf = render(&img, &Edits { rot90: 1, flip_h: true, ..Edits::default() });
    assert_eq!(px(&rf, 0, 0), px(&img, 0, 0));
}

#[test]
fn crop_in_whole_pixels() {
    let img = pattern(100, 50);
    let e = Edits { crop: Crop { l: 0.25, t: 0.2, r: 0.75, b: 0.6 }, ..Edits::default() };
    let out = render(&img, &e);
    assert_eq!(out.dimensions(), (50, 20));
    assert_eq!(px(&out, 0, 0), px(&img, 25, 10));
    assert_eq!(px(&out, 49, 19), px(&img, 74, 29));
    // Crop applies to the turned picture.
    let e = Edits { rot90: 1, crop: Crop { l: 0.0, t: 0.0, r: 0.5, b: 1.0 }, ..Edits::default() };
    assert_eq!(render(&img, &e).dimensions(), (25, 100));
}

#[test]
fn straighten_size_and_sampling() {
    // A square turned 45°: the largest inner square has side w / √2.
    let e = Edits { angle: 45.0, ..Edits::default() };
    assert_eq!(output_size(200, 200, &e), (141, 141));
    // 30° on 400×300: the general case of the inner-rectangle formula.
    let e30 = Edits { angle: 30.0, ..Edits::default() };
    let (w, h) = output_size(400, 300, &e30);
    let (s, c) = (30f64.to_radians().sin(), 30f64.to_radians().cos());
    let short = 300.0;
    assert!(short <= 2.0 * s * c * 400.0); // half-constrained case
    assert!((w as f64 - 150.0 / s).abs() <= 1.0 && (h as f64 - 150.0 / c).abs() <= 1.0, "{w}x{h}");
    // A flat colour stays flat.
    let out = render(&flat(80, 60, [10, 200, 90]), &Edits { angle: 7.5, ..Edits::default() });
    assert!(out.pixels().all(|p| p.0 == [10, 200, 90, 255]));
    // A ramp along x: bilinear sampling reproduces a straight ramp, turned by the angle.
    // Clockwise by θ: one pixel right in the output = cos θ further right in the source;
    // one pixel down = sin θ further right.
    let ramp = RgbaImage::from_fn(201, 201, |x, _| Rgba([x as u8, 0, 0, 255]));
    let a = 10f32;
    let out = render(&ramp, &Edits { angle: a, ..Edits::default() });
    let (ow, oh) = out.dimensions();
    let (cx, cy) = (ow / 2, oh / 2);
    let at = |dx: i32, dy: i32| px(&out, (cx as i32 + dx) as u32, (cy as i32 + dy) as u32)[0] as f32;
    let (s, c) = (a.to_radians().sin(), a.to_radians().cos());
    assert!((at(40, 0) - at(0, 0) - 40.0 * c).abs() <= 1.0, "{} {}", at(40, 0), at(0, 0));
    assert!((at(0, 40) - at(0, 0) - 40.0 * s).abs() <= 1.0, "{} {}", at(0, 40), at(0, 0));
}

#[test]
fn output_size_matches_render() {
    let img = pattern(97, 61);
    for rot90 in 0..4 {
        for angle in [0.0, -44.9, -3.3, 0.01, 12.0, 45.0] {
            for crop in [Crop::FULL, Crop { l: 0.13, t: 0.0, r: 0.5, b: 0.77 }, Crop { l: 0.99, t: 0.99, r: 1.0, b: 1.0 }] {
                let e = Edits { rot90, angle, crop, flip_h: rot90 == 2, ..Edits::default() };
                assert_eq!(render(&img, &e).dimensions(), output_size(97, 61, &e), "{e:?}");
            }
        }
    }
}

#[test]
fn exposure_is_linear_light() {
    // +1 EV doubles linear light: sRGB 128 (0.2159 linear) → 0.4317 linear → sRGB 175.6 → 176;
    // -1 EV: 0.1079 linear → sRGB 92.4 → 92.
    let out = render(&flat(4, 4, [128, 128, 128]), &Edits { exposure: 1.0, ..Edits::default() });
    assert_eq!(px(&out, 0, 0), [176, 176, 176, 255]);
    let dark = render(&flat(4, 4, [128, 128, 128]), &Edits { exposure: -1.0, ..Edits::default() });
    assert_eq!(px(&dark, 0, 0)[0], 92);
    // Black stays black, white clips at white, alpha is kept.
    let mut img = flat(2, 1, [0, 0, 0]);
    img.put_pixel(1, 0, Rgba([255, 255, 255, 77]));
    let out = render(&img, &Edits { exposure: 3.0, ..Edits::default() });
    assert_eq!(px(&out, 0, 0), [0, 0, 0, 255]);
    assert_eq!(px(&out, 1, 0), [255, 255, 255, 77]);
}

#[test]
fn contrast_s_curve_around_mid_grey() {
    let ramp = RgbaImage::from_fn(256, 1, |x, _| Rgba([x as u8, x as u8, x as u8, 255]));
    let up = render(&ramp, &Edits { contrast: 100, ..Edits::default() });
    let down = render(&ramp, &Edits { contrast: -100, ..Edits::default() });
    // y < 0.5 → 2y², y > 0.5 → 1 - 2(1-y)²: 64/255 → 32, 128 → ~128, 192 → 224.
    assert_eq!(px(&up, 64, 0)[0], 32);
    assert!((px(&up, 128, 0)[0] as i32 - 128).abs() <= 1);
    assert_eq!(px(&up, 192, 0)[0], 224);
    assert!(px(&down, 64, 0)[0] > 64 && px(&down, 192, 0)[0] < 192);
    assert_eq!(px(&up, 0, 0)[0], 0);
    assert_eq!(px(&up, 255, 0)[0], 255);
}

#[test]
fn highlights_and_shadows_act_on_their_own_range() {
    let ramp = RgbaImage::from_fn(256, 1, |x, _| Rgba([x as u8, x as u8, x as u8, 255]));
    let h = render(&ramp, &Edits { highlights: -100, ..Edits::default() });
    assert!(px(&h, 240, 0)[0] < 220, "highlights pulled down");
    assert_eq!(px(&h, 60, 0)[0], 60, "shadows untouched");
    let s = render(&ramp, &Edits { shadows: 100, ..Edits::default() });
    assert!(px(&s, 30, 0)[0] > 60, "shadows lifted");
    assert_eq!(px(&s, 200, 0)[0], 200, "highlights untouched");
}

#[test]
fn temperature_tint_saturation_vibrance() {
    let grey = flat(2, 2, [128, 128, 128]);
    let warm = px(&render(&grey, &Edits { temperature: 100, ..Edits::default() }), 0, 0);
    assert!(warm[0] > 128 && warm[2] < 128 && warm[1] == 128, "{warm:?}");
    let cool = px(&render(&grey, &Edits { temperature: -100, ..Edits::default() }), 0, 0);
    assert!(cool[0] < 128 && cool[2] > 128, "{cool:?}");
    let magenta = px(&render(&grey, &Edits { tint: 100, ..Edits::default() }), 0, 0);
    assert!(magenta[1] < 128 && magenta[0] > 128, "{magenta:?}");

    let red = flat(2, 2, [200, 60, 60]);
    let bw = px(&render(&red, &Edits { saturation: -100, ..Edits::default() }), 0, 0);
    assert!(bw[0] == bw[1] && bw[1] == bw[2], "{bw:?}");
    let more = px(&render(&red, &Edits { saturation: 50, ..Edits::default() }), 0, 0);
    assert!(more[0] > 200 && more[1] < 60, "{more:?}");
    // Vibrance boosts a muted colour more than a strong one.
    let muted = flat(1, 1, [140, 120, 120]);
    let strong = flat(1, 1, [250, 20, 20]);
    let v = Edits { vibrance: 100, ..Edits::default() };
    let dm = px(&render(&muted, &v), 0, 0)[0] as i32 - 140;
    let ds = px(&render(&strong, &v), 0, 0)[0] as i32 - 250;
    assert!(dm > 0 && ds <= 5 && dm > ds, "muted +{dm}, strong +{ds}");
}

#[test]
fn filters_blend_by_amount() {
    let c = flat(2, 2, [200, 120, 40]);
    let full = |name| px(&render(&c, &Edits { filter: Some(Filter { name, amount: 1.0 }), ..Edits::default() }), 0, 0);
    let bw = full(FilterName::Bw);
    // 0.299·200 + 0.587·120 + 0.114·40 = 135.16
    assert_eq!(bw, [135, 135, 135, 255]);
    let sepia = full(FilterName::Sepia);
    assert!(sepia[0] >= sepia[1] && sepia[1] >= sepia[2]);
    assert!(full(FilterName::Warm)[2] < 40);
    assert!(full(FilterName::Cool)[0] < 200);
    let fade = full(FilterName::Fade);
    assert!(fade[2] > 40 && fade[0] < 200);
    let vivid = full(FilterName::Vivid);
    assert!(vivid[0] > 200 && vivid[2] < 40);
    for name in FilterName::ALL {
        let half = px(&render(&c, &Edits { filter: Some(Filter { name, amount: 0.5 }), ..Edits::default() }), 0, 0);
        let f = full(name);
        for k in 0..3 {
            let mid = (c.get_pixel(0, 0).0[k] as f32 + f[k] as f32) / 2.0;
            assert!((half[k] as f32 - mid).abs() <= 1.0, "{name:?} half {half:?} full {f:?}");
        }
    }
}

/// `cargo test --release -p starbase-photo-edit -- --ignored --nocapture perf`
#[test]
#[ignore = "timing only: run by hand with --release"]
fn perf() {
    let full = Edits {
        angle: 3.5,
        crop: Crop { l: 0.05, t: 0.05, r: 0.95, b: 0.95 },
        exposure: 0.4,
        contrast: 25,
        highlights: -30,
        shadows: 30,
        saturation: 10,
        vibrance: 20,
        temperature: 10,
        tint: 5,
        filter: Some(Filter { name: FilterName::Vivid, amount: 0.5 }),
        ..Edits::default()
    };
    let tone = Edits { angle: 0.0, crop: Crop::FULL, ..full.clone() };
    for (w, h) in [(1024, 768), (4000, 3000)] {
        let img = pattern(w, h);
        for (label, e) in [("full edit", &full), ("tone only", &tone)] {
            let _ = render(&img, e);
            let t = std::time::Instant::now();
            let n = 5;
            for _ in 0..n {
                std::hint::black_box(render(&img, e));
            }
            println!("{w}x{h} {label}: {:.1} ms", t.elapsed().as_secs_f64() * 1000.0 / n as f64);
        }
    }
}

// ---- auto enhance and red-eye ----

fn luma_spread(img: &RgbaImage) -> (f32, f32) {
    let v: Vec<f32> = img.pixels().map(|p| 0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32).collect();
    let mean = v.iter().sum::<f32>() / v.len() as f32;
    let sd = (v.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / v.len() as f32).sqrt();
    (sd, v.iter().cloned().fold(0.0, f32::max) - v.iter().cloned().fold(255.0, f32::min))
}

/// A dull picture: a ramp squeezed into 90..170 with a blue-ish cast.
fn dull() -> RgbaImage {
    RgbaImage::from_fn(64, 64, |x, y| {
        let v = 90 + ((x + y) * 80 / 126) as u8;
        Rgba([v - 10, v, v + 10, 255])
    })
}

#[test]
fn auto_widens_the_histogram_and_is_plain_slider_values() {
    let img = dull();
    let e = starbase_photo_edit::auto_enhance(&img, &Edits::default());
    assert!(e.exposure != 0.0 || e.contrast != 0, "{e:?}");
    assert_eq!(e, starbase_photo_edit::auto_enhance(&img, &Edits::default()), "deterministic");
    let out = render(&img, &e);
    let (sd0, range0) = luma_spread(&img);
    let (sd1, range1) = luma_spread(&out);
    assert!(sd1 > sd0 * 1.2 && range1 > range0 * 1.2, "sd {sd0}->{sd1} range {range0}->{range1}");
    // the blue cast is reduced
    let (a, b) = (px(&img, 32, 32), px(&out, 32, 32));
    assert!((b[2] as i32 - b[0] as i32).abs() < (a[2] as i32 - a[0] as i32).abs());
    // keeps other edits, survives JSON
    let kept = Edits { rot90: 1, saturation: 20, ..Edits::default() };
    let e2 = starbase_photo_edit::auto_enhance(&img, &kept);
    assert_eq!((e2.rot90, e2.saturation), (1, 20));
    assert_eq!(parse(&serde_json::to_string(&e2).unwrap()).unwrap(), e2);
}

#[test]
fn auto_on_flat_or_empty_pictures_changes_nothing() {
    assert!(starbase_photo_edit::auto_enhance(&flat(8, 8, [100, 100, 100]), &Edits::default()).is_identity());
    assert!(starbase_photo_edit::auto_enhance(&RgbaImage::new(0, 0), &Edits::default()).is_identity());
}

#[test]
fn red_eye_turns_a_red_disk_dark_grey_and_leaves_the_rest() {
    let mut img = flat(100, 100, [120, 110, 100]);
    for y in 0..100u32 {
        for x in 0..100u32 {
            if (x as i32 - 50).pow(2) + (y as i32 - 50).pow(2) <= 25 {
                img.put_pixel(x, y, Rgba([210, 20, 20, 255]));
            }
        }
    }
    // a red pixel outside the spot stays red
    img.put_pixel(5, 5, Rgba([210, 20, 20, 255]));
    let e = Edits { red_eye: vec![starbase_photo_edit::RedEye { x: 0.5, y: 0.5, r: 0.08 }], ..Edits::default() };
    assert!(!e.is_identity());
    let out = render(&img, &e);
    assert_eq!(px(&out, 50, 50), [20, 20, 20, 255]);
    assert_eq!(px(&out, 5, 5), [210, 20, 20, 255]);
    assert_eq!(px(&out, 80, 80), [120, 110, 100, 255], "the skin around is untouched");
    assert_eq!(parse(&serde_json::to_string(&e).unwrap()).unwrap(), e);
    // follows the eye through a rotation
    let r = render(&img, &Edits { rot90: 1, ..e.clone() });
    assert_eq!(px(&r, 50, 50), [20, 20, 20, 255]);
}

#[test]
fn red_eye_spots_are_range_checked() {
    assert!(parse(r#"{"red_eye":[{"x":2,"y":0.5,"r":0.05}]}"#).is_err());
    assert!(parse(r#"{"red_eye":[{"x":0.5,"y":0.5,"r":0}]}"#).is_err());
}
