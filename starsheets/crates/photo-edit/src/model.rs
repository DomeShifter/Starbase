//! The edit model: what the JSON in a sidecar says (contract §1, version 1).
//!
//! `Edits` is only made through [`Edits::validate`] when it comes from outside: serde decodes into a
//! private raw shape first and converts with `TryFrom` (`#[serde(try_from)]`), so untrusted JSON
//! can't skip the range checks. The fields are public for the editor's sliders; `render` clamps
//! them again, so a bad value set in code changes the picture less, never panics.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Model version written in `"v"`.
pub const VERSION: u32 = 1;
/// Smallest crop side, as a fraction of the straightened image.
pub const MIN_CROP: f32 = 0.01;
/// Straighten range in degrees (either way).
pub const MAX_ANGLE: f32 = 45.0;
/// Exposure range in EV (either way).
pub const MAX_EXPOSURE: f32 = 5.0;
/// Slider range (either way) for contrast, highlights, shadows, saturation, vibrance, temperature, tint.
pub const MAX_SLIDER: i32 = 100;

/// Crop rectangle as fractions (0..=1) of the rotated and straightened image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crop {
    pub l: f32,
    pub t: f32,
    pub r: f32,
    pub b: f32,
}

impl Crop {
    pub const FULL: Crop = Crop { l: 0.0, t: 0.0, r: 1.0, b: 1.0 };

    pub fn is_full(&self) -> bool {
        *self == Crop::FULL
    }
}

impl Default for Crop {
    fn default() -> Self {
        Crop::FULL
    }
}

/// The crop tool's aspect choice. A hint for the editor only: it never changes the pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Aspect {
    #[serde(rename = "free")]
    Free,
    #[serde(rename = "original")]
    Original,
    #[serde(rename = "1:1")]
    Square,
    #[serde(rename = "4:3")]
    R4x3,
    #[serde(rename = "3:2")]
    R3x2,
    #[serde(rename = "16:9")]
    R16x9,
    #[serde(rename = "4:5")]
    R4x5,
}

/// A one-click look.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilterName {
    Bw,
    Warm,
    Cool,
    Vivid,
    Fade,
    Sepia,
}

impl FilterName {
    pub const ALL: [FilterName; 6] = [FilterName::Bw, FilterName::Warm, FilterName::Cool, FilterName::Vivid, FilterName::Fade, FilterName::Sepia];
}

/// A filter and how strongly it is blended in (0 = off, 1 = full).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Filter {
    pub name: FilterName,
    pub amount: f32,
}

/// Most red-eye spots on one photo.
pub const MAX_RED_EYE: usize = 64;

/// A red-eye spot: centre as fractions of the upright source picture, radius as a fraction of its
/// shorter side. Applied before rotating and cropping, so it stays on the eye.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RedEye {
    pub x: f32,
    pub y: f32,
    pub r: f32,
}

/// One photo's edits. `Edits::default()` changes nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawEdits", into = "RawEdits")]
pub struct Edits {
    /// Quarter turns clockwise, 0..=3, applied first.
    pub rot90: u8,
    /// Mirror left-right, after `rot90`.
    pub flip_h: bool,
    /// Straighten in degrees, -45..=45, positive = clockwise; crops to the largest inner rectangle.
    pub angle: f32,
    pub crop: Crop,
    pub aspect: Option<Aspect>,
    /// EV, -5..=5, in linear light.
    pub exposure: f32,
    pub contrast: i32,
    pub highlights: i32,
    pub shadows: i32,
    pub saturation: i32,
    pub vibrance: i32,
    /// Positive = warmer.
    pub temperature: i32,
    /// Positive = magenta, negative = green (as Lightroom).
    pub tint: i32,
    pub filter: Option<Filter>,
    /// Red-eye spots, applied first (on the source frame).
    pub red_eye: Vec<RedEye>,
}

impl Default for Edits {
    fn default() -> Self {
        Edits {
            rot90: 0,
            flip_h: false,
            angle: 0.0,
            crop: Crop::FULL,
            aspect: None,
            exposure: 0.0,
            contrast: 0,
            highlights: 0,
            shadows: 0,
            saturation: 0,
            vibrance: 0,
            temperature: 0,
            tint: 0,
            filter: None,
            red_eye: Vec::new(),
        }
    }
}

impl Edits {
    /// True when rendering gives back the same pixels (the aspect hint doesn't count).
    pub fn is_identity(&self) -> bool {
        self.geometry_is_identity() && self.pixels_are_identity() && self.red_eye.is_empty()
    }

    /// No rotation, flip, straighten or crop.
    pub fn geometry_is_identity(&self) -> bool {
        self.rot90.is_multiple_of(4) && !self.flip_h && self.angle == 0.0 && self.crop.is_full()
    }

    /// No tone, colour or filter change.
    pub fn pixels_are_identity(&self) -> bool {
        self.exposure == 0.0
            && self.contrast == 0
            && self.highlights == 0
            && self.shadows == 0
            && self.saturation == 0
            && self.vibrance == 0
            && self.temperature == 0
            && self.tint == 0
            && self.filter.is_none_or(|f| f.amount == 0.0)
    }

    /// Checks every range (the same checks JSON input gets).
    pub fn validate(&self) -> Result<(), EditError> {
        if self.rot90 > 3 {
            return Err(EditError::new("rot90 must be 0..=3"));
        }
        check_f("angle", self.angle as f64, -MAX_ANGLE as f64, MAX_ANGLE as f64)?;
        check_f("exposure", self.exposure as f64, -MAX_EXPOSURE as f64, MAX_EXPOSURE as f64)?;
        let c = self.crop;
        for (name, v) in [("crop.l", c.l), ("crop.t", c.t), ("crop.r", c.r), ("crop.b", c.b)] {
            check_f(name, v as f64, 0.0, 1.0)?;
        }
        // A little slack: f32 values written as JSON and read back can lose the last bit.
        let slack = 1e-6;
        if (c.r as f64 - c.l as f64) < MIN_CROP as f64 - slack || (c.b as f64 - c.t as f64) < MIN_CROP as f64 - slack {
            return Err(EditError::new("crop must have l < r and t < b, each side at least 0.01"));
        }
        for (name, v) in [
            ("contrast", self.contrast),
            ("highlights", self.highlights),
            ("shadows", self.shadows),
            ("saturation", self.saturation),
            ("vibrance", self.vibrance),
            ("temperature", self.temperature),
            ("tint", self.tint),
        ] {
            if !(-MAX_SLIDER..=MAX_SLIDER).contains(&v) {
                return Err(EditError(format!("{name} must be -100..=100")));
            }
        }
        if self.red_eye.len() > MAX_RED_EYE {
            return Err(EditError(format!("at most {MAX_RED_EYE} red-eye spots")));
        }
        for s in &self.red_eye {
            check_f("red_eye.x", s.x as f64, 0.0, 1.0)?;
            check_f("red_eye.y", s.y as f64, 0.0, 1.0)?;
            check_f("red_eye.r", s.r as f64, 0.001, 0.5)?;
        }
        if let Some(f) = self.filter {
            check_f("filter.amount", f.amount as f64, 0.0, 1.0)?;
        }
        Ok(())
    }
}

fn check_f(name: &str, v: f64, lo: f64, hi: f64) -> Result<(), EditError> {
    if !v.is_finite() {
        return Err(EditError(format!("{name} must be a finite number")));
    }
    if v < lo || v > hi {
        return Err(EditError(format!("{name} must be {lo}..={hi}")));
    }
    Ok(())
}

/// Why edits were refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditError(pub String);

impl EditError {
    fn new(s: &str) -> Self {
        EditError(s.to_owned())
    }
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid photo edits: {}", self.0)
    }
}

impl std::error::Error for EditError {}

// ---- wire shape: every field optional (null = absent = no change) ----

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct RawCrop {
    l: Option<f64>,
    t: Option<f64>,
    r: Option<f64>,
    b: Option<f64>,
}

#[derive(Serialize, Deserialize)]
struct RawFilter {
    name: FilterName,
    #[serde(default)]
    amount: Option<f64>,
}

#[derive(Serialize, Deserialize)]
struct RawRedEye {
    x: f64,
    y: f64,
    r: f64,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct RawEdits {
    v: Option<u64>,
    rot90: Option<i64>,
    flip_h: Option<bool>,
    angle: Option<f64>,
    crop: Option<RawCrop>,
    aspect: Option<Aspect>,
    exposure: Option<f64>,
    contrast: Option<i64>,
    highlights: Option<i64>,
    shadows: Option<i64>,
    saturation: Option<i64>,
    vibrance: Option<i64>,
    temperature: Option<i64>,
    tint: Option<i64>,
    filter: Option<RawFilter>,
    red_eye: Option<Vec<RawRedEye>>,
}

/// f32 → f64 by its shortest decimal form, so 0.1 is written as 0.1, not 0.10000000149011612.
fn wide(x: f32) -> f64 {
    x.to_string().parse().unwrap_or(0.0)
}

/// Range-checks in f64 before narrowing, so 1e300 is refused rather than turned into infinity.
fn f(name: &str, v: Option<f64>, default: f32, lo: f32, hi: f32) -> Result<f32, EditError> {
    match v {
        None => Ok(default),
        Some(v) => {
            check_f(name, v, lo as f64, hi as f64)?;
            Ok(v as f32)
        }
    }
}

fn slider(name: &str, v: Option<i64>) -> Result<i32, EditError> {
    match v {
        None => Ok(0),
        Some(v) if (-(MAX_SLIDER as i64)..=MAX_SLIDER as i64).contains(&v) => Ok(v as i32),
        Some(_) => Err(EditError(format!("{name} must be -100..=100"))),
    }
}

impl TryFrom<RawEdits> for Edits {
    type Error = EditError;

    fn try_from(r: RawEdits) -> Result<Self, EditError> {
        if let Some(v) = r.v
            && v != VERSION as u64
        {
            return Err(EditError(format!("unsupported version {v} (this program reads version {VERSION})")));
        }
        let rot90 = match r.rot90 {
            None => 0,
            Some(n @ 0..=3) => n as u8,
            Some(_) => return Err(EditError::new("rot90 must be 0..=3")),
        };
        let rc = r.crop.unwrap_or_default();
        let crop = Crop {
            l: f("crop.l", rc.l, 0.0, 0.0, 1.0)?,
            t: f("crop.t", rc.t, 0.0, 0.0, 1.0)?,
            r: f("crop.r", rc.r, 1.0, 0.0, 1.0)?,
            b: f("crop.b", rc.b, 1.0, 0.0, 1.0)?,
        };
        let filter = match r.filter {
            None => None,
            Some(rf) => Some(Filter { name: rf.name, amount: f("filter.amount", rf.amount, 1.0, 0.0, 1.0)? }),
        };
        let raw_eyes = r.red_eye.unwrap_or_default();
        if raw_eyes.len() > MAX_RED_EYE {
            return Err(EditError(format!("at most {MAX_RED_EYE} red-eye spots")));
        }
        let mut red_eye = Vec::with_capacity(raw_eyes.len());
        for s in raw_eyes {
            red_eye.push(RedEye {
                x: f("red_eye.x", Some(s.x), 0.0, 0.0, 1.0)?,
                y: f("red_eye.y", Some(s.y), 0.0, 0.0, 1.0)?,
                r: f("red_eye.r", Some(s.r), 0.0, 0.001, 0.5)?,
            });
        }
        let e = Edits {
            rot90,
            flip_h: r.flip_h.unwrap_or(false),
            angle: f("angle", r.angle, 0.0, -MAX_ANGLE, MAX_ANGLE)?,
            crop,
            aspect: r.aspect,
            exposure: f("exposure", r.exposure, 0.0, -MAX_EXPOSURE, MAX_EXPOSURE)?,
            contrast: slider("contrast", r.contrast)?,
            highlights: slider("highlights", r.highlights)?,
            shadows: slider("shadows", r.shadows)?,
            saturation: slider("saturation", r.saturation)?,
            vibrance: slider("vibrance", r.vibrance)?,
            temperature: slider("temperature", r.temperature)?,
            tint: slider("tint", r.tint)?,
            filter,
            red_eye,
        };
        e.validate()?;
        Ok(e)
    }
}

impl From<Edits> for RawEdits {
    fn from(e: Edits) -> Self {
        RawEdits {
            v: Some(VERSION as u64),
            rot90: Some(e.rot90 as i64),
            flip_h: Some(e.flip_h),
            angle: Some(wide(e.angle)),
            crop: Some(RawCrop { l: Some(wide(e.crop.l)), t: Some(wide(e.crop.t)), r: Some(wide(e.crop.r)), b: Some(wide(e.crop.b)) }),
            aspect: e.aspect,
            exposure: Some(wide(e.exposure)),
            contrast: Some(e.contrast as i64),
            highlights: Some(e.highlights as i64),
            shadows: Some(e.shadows as i64),
            saturation: Some(e.saturation as i64),
            vibrance: Some(e.vibrance as i64),
            temperature: Some(e.temperature as i64),
            tint: Some(e.tint as i64),
            filter: e.filter.map(|f| RawFilter { name: f.name, amount: Some(wide(f.amount)) }),
            red_eye: if e.red_eye.is_empty() {
                None
            } else {
                Some(e.red_eye.iter().map(|s| RawRedEye { x: wide(s.x), y: wide(s.y), r: wide(s.r) }).collect())
            },
        }
    }
}
