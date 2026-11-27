//! The screen copies' formats, best first, and the JPEG XL encoder.
//!
//! JPEG XL is made by the external `cjxl` program (libjxl, BSD-3; Debian/Ubuntu package
//! `libjxl-tools`), run as a separate process: the Rust bindings to libjxl are GPL-3.0, which
//! this MIT/Apache server can't link. When `cjxl` isn't installed the server says so once in its
//! log and serves JPEG screen copies only.
//!
//! A future better format is one more entry in [`FORMATS`] (and its encoder here).

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// A screen copy's format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Jxl,
    Jpeg,
}

/// Formats in order of preference; the last one every browser shows.
pub const FORMATS: [Format; 2] = [Format::Jxl, Format::Jpeg];

/// Effort (1-10) and distance (0 = lossless; 1 = visually lossless) of `cjxl`. e7 d2 measured
/// at about SSIMULACRA2 80, ~335 KB for a 2560 px photo, 0.4 s.
const EFFORT: &str = "7";
const DISTANCE: &str = "2.0";
/// Threads one encode may use (it runs in the background next to everything else).
const THREADS: &str = "4";
/// An encode taking longer than this is stopped.
const TIMEOUT: Duration = Duration::from_secs(60);

/// Whether `cjxl` can be run (looked at once; says so in the log when it can't).
pub fn available() -> bool {
    static HAVE: OnceLock<bool> = OnceLock::new();
    *HAVE.get_or_init(|| {
        if std::env::var_os("STARBASE_PHOTOS_NO_JXL").is_some() {
            return false;
        }
        let ok = Command::new("cjxl")
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            eprintln!("photos: `cjxl` not found (package libjxl-tools): screen copies will be JPEG only");
        }
        ok
    })
}

/// The best format a browser that can show `best` (it said `?fmt=best`) gets.
pub fn best(browser_best: bool) -> Format {
    FORMATS
        .into_iter()
        .find(|f| match f {
            Format::Jxl => browser_best && available(),
            Format::Jpeg => true,
        })
        .unwrap_or(Format::Jpeg)
}

/// Encodes a PPM or PNG (`ext`) as JPEG XL, using `dir` for the two scratch files. There is no
/// colour profile in the input (the child hands over plain pixels), so it is encoded as sRGB.
pub fn encode(input: &[u8], ext: &str, dir: &Path) -> Result<Vec<u8>, String> {
    if !matches!(ext, "ppm" | "png") {
        return Err(format!("cjxl input must be ppm or png, not {ext}"));
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let stem = format!("screen-{}", crate::store::random_hex(6));
    let (src, dst) = (dir.join(format!("{stem}.{ext}")), dir.join(format!("{stem}.jxl")));
    let r = (|| {
        std::fs::File::create(&src).and_then(|mut f| f.write_all(input)).map_err(|e| e.to_string())?;
        let mut child = Command::new("cjxl")
            .arg(&src)
            .arg(&dst)
            .args(["-e", EFFORT, "-d", DISTANCE, "--num_threads", THREADS, "--quiet"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("cjxl: {e}"))?;
        let t = Instant::now();
        loop {
            match child.try_wait().map_err(|e| e.to_string())? {
                Some(s) if s.success() => break,
                Some(s) => return Err(format!("cjxl failed ({s})")),
                None if t.elapsed() > TIMEOUT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("cjxl took too long".into());
                }
                None => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        std::fs::read(&dst).map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&dst);
    r
}
