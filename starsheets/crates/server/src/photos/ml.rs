//! The face worker (Photos roadmap item 7): one background thread at low priority that looks
//! for faces in the pictures of the photo index and keeps their boxes and embeddings in the
//! index (faces.rs), then groups them. Off until turned on.
//!
//! - **On/off**: `STARBASE_PHOTOS_FACES=on|off` decides; without it, the server's owner turns it on
//!   in Photos › People (`POST /api/photos/faces {"on": true}`, kept in the index). Off by default.
//! - **Models** (ONNX) are never in the program: they are looked for in `STARBASE_ML_MODELS`
//!   (default `<photos cache>/models`). See [`REGISTRY`] for the file names, licences and where to
//!   download them. Missing files are named in Photos › People for the owner. The embedder is
//!   `STARBASE_FACES_MODEL=auraface` (default, better) or `sface` (much faster, for weak CPUs).
//! - **Input**: the picture's own 1024 px thumbnail, already decoded (in a child process, within
//!   limits) and turned upright by the thumbnail worker: no picture file is decoded here.
//!   Detection is YuNet on that thumbnail (the trial found 720 px and 1280 px+tiles find the same
//!   faces; 1280 px is a follow-up).
//! - **Priority**: low CPU and idle I/O priority, one thread, `STARBASE_ML_THREADS` ONNX threads
//!   (default a quarter of the cores); waits while thumbnails are being made. Resumable: each
//!   picture looked at is noted in the index (`faces_done`).
//! - The ONNX Runtime is only in builds with the `ml` feature (`cargo build --features ml`);
//!   other builds say so in the People view.

// The pure helpers (YuNet decoding, alignment, model inputs) are only called by the ONNX engine,
// which is in `--features ml` builds; the tests use them in every build.
#![cfg_attr(not(feature = "ml"), allow(dead_code))]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use super::faces;
use super::Photos;

/// A model file the worker uses.
pub struct ModelFile {
    pub file: &'static str,
    pub what: &'static str,
    pub licence: &'static str,
    pub url: &'static str,
}

/// The model registry: only permissively licensed models (photos-mode.md §7).
pub const REGISTRY: [(&str, ModelFile); 3] = [
    (
        "yunet",
        ModelFile {
            file: "face_detection_yunet_2023mar.onnx",
            what: "YuNet face detector",
            licence: "MIT",
            url: "https://github.com/opencv/opencv_zoo/tree/main/models/face_detection_yunet",
        },
    ),
    (
        "auraface",
        ModelFile {
            file: "glintr100.onnx",
            what: "AuraFace v1 face embedder",
            licence: "Apache-2.0",
            url: "https://huggingface.co/fal/AuraFace-v1 (only glintr100.onnx: not the SCRFD file beside it)",
        },
    ),
    (
        "sface",
        ModelFile {
            file: "face_recognition_sface_2021dec.onnx",
            what: "SFace face embedder",
            licence: "Apache-2.0",
            url: "https://github.com/opencv/opencv_zoo/tree/main/models/face_recognition_sface",
        },
    ),
];

pub fn model_file(name: &str) -> Option<&'static ModelFile> {
    REGISTRY.iter().find(|(n, _)| *n == name).map(|(_, m)| m)
}

/// The worker's settings (from the environment).
#[derive(Clone, Debug)]
pub struct MlConfig {
    /// `Some(on)` when the environment decides; `None`: the owner's toggle.
    pub forced: Option<bool>,
    pub models: Option<PathBuf>,
    /// "auraface" or "sface".
    pub embedder: String,
    pub threads: usize,
}

impl MlConfig {
    pub fn from_env() -> MlConfig {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let forced = var("STARBASE_PHOTOS_FACES").map(|v| matches!(v.trim(), "on" | "1" | "true" | "yes"));
        let embedder = match var("STARBASE_FACES_MODEL").as_deref().map(str::trim) {
            Some("sface") => "sface",
            _ => "auraface",
        };
        let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
        let threads = var("STARBASE_ML_THREADS").and_then(|v| v.trim().parse().ok()).unwrap_or((cores / 4).max(1)).clamp(1, 64);
        MlConfig { forced, models: var("STARBASE_ML_MODELS").map(PathBuf::from), embedder: embedder.into(), threads }
    }
}

/// The worker's state, shared with the HTTP side.
pub struct State {
    pub cfg: MlConfig,
    /// What the worker is doing, or why it can't (for the People view).
    pub note: Mutex<String>,
    /// Faces changed by someone: group again soon.
    pub dirty: AtomicBool,
    wake: Condvar,
    sleep: Mutex<()>,
}

impl State {
    pub fn new(cfg: MlConfig) -> State {
        State { cfg, note: Mutex::new("Not started.".into()), dirty: AtomicBool::new(false), wake: Condvar::new(), sleep: Mutex::new(()) }
    }
    /// Wakes the worker (turned on, or faces changed).
    pub fn poke(&self) {
        self.wake.notify_all();
    }
    fn nap(&self, d: Duration) {
        let g = self.sleep.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self.wake.wait_timeout(g, d);
    }
    fn say(&self, s: impl Into<String>) {
        *self.note.lock().unwrap_or_else(|e| e.into_inner()) = s.into();
    }
    pub fn note(&self) -> String {
        self.note.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Photos {
    /// The folder the models are looked for in.
    pub fn models_dir(&self) -> PathBuf {
        self.ml.cfg.models.clone().unwrap_or_else(|| self.cache_dir().join("models"))
    }

    /// The embedding model in use ("auraface" or "sface").
    pub fn face_model(&self) -> &str {
        &self.ml.cfg.embedder
    }

    /// Whether face recognition is on.
    pub fn faces_on(&self) -> bool {
        self.ml.cfg.forced.unwrap_or_else(|| self.with_db(|ix| faces::setting(ix, "faces").as_deref() == Some("on")))
    }

    /// The model files that are missing.
    pub fn missing_models(&self) -> Vec<&'static ModelFile> {
        let dir = self.models_dir();
        ["yunet", self.face_model()].iter().filter_map(|n| model_file(n)).filter(|m| !dir.join(m.file).is_file()).collect()
    }

    /// The face worker thread.
    pub(super) fn face_worker(self: Arc<Self>) {
        super::low_priority();
        let model = self.face_model().to_string();
        let mut engine: Option<Box<dyn Engine>> = None;
        loop {
            if self.stopped() {
                return;
            }
            if !self.faces_on() {
                engine = None;
                self.ml.say("Off.");
                self.ml.nap(Duration::from_secs(30));
                continue;
            }
            if self.ml.dirty.swap(false, Ordering::Relaxed) {
                let _ = faces::regroup(self.db_lock(), &model);
            }
            if engine.is_none() {
                let missing = self.missing_models();
                if !missing.is_empty() {
                    self.ml.say(format!(
                        "Model files missing in {}: {}.",
                        self.models_dir().display(),
                        missing.iter().map(|m| format!("{} ({}, {}, from {})", m.file, m.what, m.licence, m.url)).collect::<Vec<_>>().join("; ")
                    ));
                    self.ml.nap(Duration::from_secs(60));
                    continue;
                }
                match load_engine(&self.models_dir(), &model, self.ml.cfg.threads) {
                    Ok(e) => engine = Some(e),
                    Err(e) => {
                        self.ml.say(e);
                        self.ml.nap(Duration::from_secs(300));
                        continue;
                    }
                }
            }
            // Thumbnails first: wait while they are being made.
            let (q, w) = self.queued();
            if q + w > 0 {
                self.ml.say("Waiting for thumbnails to be made.");
                self.ml.nap(Duration::from_secs(5));
                continue;
            }
            let todo = self.with_db(|ix| faces::todo(ix, &model, 16).unwrap_or_default());
            if todo.is_empty() {
                self.ml.say("Up to date.");
                self.ml.nap(Duration::from_secs(60));
                continue;
            }
            self.ml.say("Looking for faces.");
            let Some(eng) = engine.as_mut() else { continue };
            for thumb in todo {
                if self.stopped() || !self.faces_on() {
                    break;
                }
                let found = match self.thumb_file(&thumb, 1024).and_then(|(p, _)| std::fs::read(p).ok()) {
                    Some(bytes) => look(eng.as_mut(), &bytes).unwrap_or_default(),
                    // No thumbnail (cleared from the cache): noted as done, it comes back if remade.
                    None => Vec::new(),
                };
                self.with_db(|ix| faces::keep(ix, &thumb, &model, &found)).ok();
            }
            let _ = faces::regroup(self.db_lock(), &model);
        }
    }
}

/// Finds faces in a thumbnail (JPEG/PNG bytes) and embeds them.
fn look(eng: &mut dyn Engine, bytes: &[u8]) -> Result<Vec<faces::Found>, String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgb8();
    let (w, h) = img.dimensions();
    let mut out = Vec::new();
    for d in eng.detect(&img)? {
        let crop = align(&img, &d.kps);
        let mut emb = eng.embed(&crop)?;
        let n = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
        if n > 0.0 {
            emb.iter_mut().for_each(|x| *x /= n);
        }
        out.push(faces::Found {
            x: d.x / w as f32,
            y: d.y / h as f32,
            w: d.w / w as f32,
            h: d.h / h as f32,
            score: d.score,
            px: d.w.min(d.h),
            emb,
        });
    }
    Ok(out)
}

/// A face the detector found, in pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Det {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub score: f32,
    /// Eyes (right, left as seen), nose, mouth corners.
    pub kps: [[f32; 2]; 5],
}

/// A face detector + embedder.
pub trait Engine: Send {
    fn detect(&mut self, img: &image::RgbImage) -> Result<Vec<Det>, String>;
    /// The embedding of a 112×112 aligned face.
    fn embed(&mut self, face: &image::RgbImage) -> Result<Vec<f32>, String>;
}

/// The lowest detection score kept.
pub const MIN_SCORE: f32 = 0.7;

/// YuNet's raw outputs for one stride: class, objectness, box (4) and landmarks (10) per cell.
pub struct YuStride<'a> {
    pub stride: usize,
    pub cls: &'a [f32],
    pub obj: &'a [f32],
    pub bbox: &'a [f32],
    pub kps: &'a [f32],
}

/// Decodes YuNet (2023mar) outputs for an input of `w`×`h` (multiples of 32), as OpenCV's
/// FaceDetectorYN does, then drops overlapping boxes (IoU > 0.3).
pub fn yunet_decode(strides: &[YuStride], w: usize, h: usize, min_score: f32) -> Vec<Det> {
    let mut out = Vec::new();
    for s in strides {
        let (cols, rows) = (w / s.stride, h / s.stride);
        let st = s.stride as f32;
        for r in 0..rows {
            for c in 0..cols {
                let i = r * cols + c;
                if i >= s.cls.len() || i >= s.obj.len() || i * 4 + 3 >= s.bbox.len() || i * 10 + 9 >= s.kps.len() {
                    continue;
                }
                let score = (s.cls[i].clamp(0.0, 1.0) * s.obj[i].clamp(0.0, 1.0)).sqrt();
                if score < min_score {
                    continue;
                }
                let cx = (c as f32 + s.bbox[i * 4]) * st;
                let cy = (r as f32 + s.bbox[i * 4 + 1]) * st;
                let bw = s.bbox[i * 4 + 2].exp() * st;
                let bh = s.bbox[i * 4 + 3].exp() * st;
                let mut kps = [[0f32; 2]; 5];
                for (n, k) in kps.iter_mut().enumerate() {
                    *k = [(s.kps[i * 10 + 2 * n] + c as f32) * st, (s.kps[i * 10 + 2 * n + 1] + r as f32) * st];
                }
                out.push(Det { x: cx - bw / 2.0, y: cy - bh / 2.0, w: bw, h: bh, score, kps });
            }
        }
    }
    nms(out, 0.3)
}

fn iou(a: &Det, b: &Det) -> f32 {
    let ix = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
    let iy = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
    if ix <= 0.0 || iy <= 0.0 {
        return 0.0;
    }
    let inter = ix * iy;
    inter / (a.w * a.h + b.w * b.h - inter)
}

/// Non-maximum suppression: the best box of each overlapping set.
pub fn nms(mut d: Vec<Det>, t: f32) -> Vec<Det> {
    d.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut keep: Vec<Det> = Vec::new();
    for x in d {
        if keep.iter().all(|k| iou(k, &x) <= t) {
            keep.push(x);
        }
    }
    keep
}

/// Where the five landmarks go in a 112×112 face (the ArcFace template, as OpenCV's alignCrop).
pub const TEMPLATE: [[f32; 2]; 5] = [[38.2946, 51.6963], [73.5318, 51.5014], [56.0252, 71.7366], [41.5493, 92.3655], [70.7299, 92.2041]];

/// The least-squares similarity transform (rotation, uniform scale, shift; no mirroring) taking
/// `src` to `dst`: `[a, b, tx, ty]` for x' = a·x − b·y + tx, y' = b·x + a·y + ty.
pub fn similarity_transform(src: &[[f32; 2]; 5], dst: &[[f32; 2]; 5]) -> [f32; 4] {
    let n = src.len() as f32;
    let ms = src.iter().fold([0.0, 0.0], |m, p| [m[0] + p[0] / n, m[1] + p[1] / n]);
    let md = dst.iter().fold([0.0, 0.0], |m, p| [m[0] + p[0] / n, m[1] + p[1] / n]);
    let (mut num_a, mut num_b, mut den) = (0.0f32, 0.0f32, 0.0f32);
    for (s, d) in src.iter().zip(dst) {
        let (sx, sy, dx, dy) = (s[0] - ms[0], s[1] - ms[1], d[0] - md[0], d[1] - md[1]);
        num_a += sx * dx + sy * dy;
        num_b += sx * dy - sy * dx;
        den += sx * sx + sy * sy;
    }
    if den == 0.0 {
        return [1.0, 0.0, md[0] - ms[0], md[1] - ms[1]];
    }
    let (a, b) = (num_a / den, num_b / den);
    [a, b, md[0] - (a * ms[0] - b * ms[1]), md[1] - (b * ms[0] + a * ms[1])]
}

/// The 112×112 face for landmarks `kps` (bilinear; black outside the picture).
pub fn align(img: &image::RgbImage, kps: &[[f32; 2]; 5]) -> image::RgbImage {
    let [a, b, tx, ty] = similarity_transform(kps, &TEMPLATE);
    let k = a * a + b * b;
    let (w, h) = (img.width() as i64, img.height() as i64);
    let px = |x: i64, y: i64| -> [f32; 3] {
        if x < 0 || y < 0 || x >= w || y >= h {
            return [0.0; 3];
        }
        let p = img.get_pixel(x as u32, y as u32).0;
        [p[0] as f32, p[1] as f32, p[2] as f32]
    };
    image::RgbImage::from_fn(112, 112, |u, v| {
        if k == 0.0 {
            return image::Rgb([0, 0, 0]);
        }
        // Back from the face to the picture: the inverse of the similarity.
        let (du, dv) = (u as f32 - tx, v as f32 - ty);
        let sx = (a * du + b * dv) / k;
        let sy = (-b * du + a * dv) / k;
        let (x0, y0) = (sx.floor() as i64, sy.floor() as i64);
        let (fx, fy) = (sx - x0 as f32, sy - y0 as f32);
        let (p00, p10, p01, p11) = (px(x0, y0), px(x0 + 1, y0), px(x0, y0 + 1), px(x0 + 1, y0 + 1));
        let mut o = [0u8; 3];
        for c in 0..3 {
            let top = p00[c] * (1.0 - fx) + p10[c] * fx;
            let bot = p01[c] * (1.0 - fx) + p11[c] * fx;
            o[c] = (top * (1.0 - fy) + bot * fy).round().clamp(0.0, 255.0) as u8;
        }
        image::Rgb(o)
    })
}

/// A picture as YuNet takes it: BGR, 0..255, NCHW, padded to multiples of 32.
pub fn yunet_input(img: &image::RgbImage) -> (usize, usize, Vec<f32>) {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let (pw, ph) = (w.div_ceil(32) * 32, h.div_ceil(32) * 32);
    let mut v = vec![0f32; 3 * pw * ph];
    for (x, y, p) in img.enumerate_pixels() {
        let i = y as usize * pw + x as usize;
        v[i] = p.0[2] as f32;
        v[pw * ph + i] = p.0[1] as f32;
        v[2 * pw * ph + i] = p.0[0] as f32;
    }
    (pw, ph, v)
}

/// A 112×112 face as an embedder takes it: RGB NCHW; AuraFace scaled to −1..1, SFace 0..255.
pub fn embed_input(face: &image::RgbImage, model: &str) -> Vec<f32> {
    let n = 112 * 112;
    let mut v = vec![0f32; 3 * n];
    for (x, y, p) in face.enumerate_pixels() {
        let i = y as usize * 112 + x as usize;
        for c in 0..3 {
            let val = p.0[c] as f32;
            v[c * n + i] = if model == "sface" { val } else { (val - 127.5) / 127.5 };
        }
    }
    v
}

#[cfg(not(feature = "ml"))]
fn load_engine(_dir: &Path, _model: &str, _threads: usize) -> Result<Box<dyn Engine>, String> {
    Err("This server was built without face recognition (the `ml` feature): faces can't be found.".into())
}

#[cfg(feature = "ml")]
fn load_engine(dir: &Path, model: &str, threads: usize) -> Result<Box<dyn Engine>, String> {
    onnx::Onnx::load(dir, model, threads).map(|e| Box::new(e) as Box<dyn Engine>)
}

#[cfg(feature = "ml")]
mod onnx {
    use std::path::Path;

    use ort::session::Session;
    use ort::value::Tensor;

    use super::{Det, Engine, MIN_SCORE, YuStride, embed_input, model_file, yunet_decode, yunet_input};

    pub struct Onnx {
        det: Session,
        emb: Session,
        model: String,
    }

    fn e(x: impl std::fmt::Display) -> String {
        format!("ONNX Runtime: {x}")
    }

    fn session(path: &Path, threads: usize) -> Result<Session, String> {
        Session::builder().map_err(e)?.with_intra_threads(threads).map_err(e)?.commit_from_file(path).map_err(e)
    }

    impl Onnx {
        pub fn load(dir: &Path, model: &str, threads: usize) -> Result<Onnx, String> {
            let f = |n: &str| model_file(n).map(|m| dir.join(m.file)).ok_or_else(|| format!("unknown model {n}"));
            Ok(Onnx { det: session(&f("yunet")?, threads)?, emb: session(&f(model)?, threads)?, model: model.to_string() })
        }
    }

    impl Engine for Onnx {
        fn detect(&mut self, img: &image::RgbImage) -> Result<Vec<Det>, String> {
            let (w, h, data) = yunet_input(img);
            let t = Tensor::from_array(([1usize, 3, h, w], data)).map_err(e)?;
            let out = self.det.run(ort::inputs![t]).map_err(e)?;
            let mut got: Vec<(usize, [Vec<f32>; 4])> = Vec::new();
            for s in [8usize, 16, 32] {
                let mut part: [Vec<f32>; 4] = Default::default();
                for (k, name) in ["cls", "obj", "bbox", "kps"].iter().enumerate() {
                    let v = out.get(format!("{name}_{s}").as_str()).ok_or_else(|| format!("YuNet: no output {name}_{s}"))?;
                    let (_, d) = v.try_extract_tensor::<f32>().map_err(e)?;
                    part[k] = d.to_vec();
                }
                got.push((s, part));
            }
            let strides: Vec<YuStride> =
                got.iter().map(|(s, p)| YuStride { stride: *s, cls: &p[0], obj: &p[1], bbox: &p[2], kps: &p[3] }).collect();
            Ok(yunet_decode(&strides, w, h, MIN_SCORE))
        }

        fn embed(&mut self, face: &image::RgbImage) -> Result<Vec<f32>, String> {
            let t = Tensor::from_array(([1usize, 3, 112, 112], embed_input(face, &self.model))).map_err(e)?;
            let out = self.emb.run(ort::inputs![t]).map_err(e)?;
            let (_, d) = out[0].try_extract_tensor::<f32>().map_err(e)?;
            Ok(d.to_vec())
        }
    }
}
