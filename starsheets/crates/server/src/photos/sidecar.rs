//! XMP sidecars (`photo.jpg.xmp`, photo-editor contract §2): where a picture's edits are kept,
//! next to it, so the original is never changed and other photo apps can read them.
//!
//! - **Reading**: `starbase:Edits` (our own namespace, the full edit as JSON) wins; a sidecar
//!   with only Adobe `crs:` fields (Lightroom, Camera Raw) is imported approximately. Other
//!   editors' history (darktable) is reported as `foreign`, so the app can warn before saving.
//! - **Writing merges, never replaces**: the fields we own (our `crs:` crop/tone/colour fields
//!   and everything `starbase:`) are cut out of the text and one new `rdf:Description` holding
//!   ours goes in before `</rdf:RDF>`. Every other byte (other namespaces, comments, packet
//!   wrappers) stays exactly as it was. The result is parsed again and must read back as the
//!   edits written, or nothing is written.
//! - **Untrusted input**: at most [`MAX_SIDECAR`] bytes are read, no DTDs (so no entity
//!   expansion: roxmltree refuses them), at most [`MAX_NODES`] nodes, UTF-8 only.
//! - **On disk**: the folder is opened once without following links and checked to be the
//!   folder asked for; the sidecar is read, written and removed relative to that open folder
//!   (`openat`, `renameat`), never through a symbolic link: a sidecar that is a link is refused.
//!   Writes go to a hidden temp file in the same folder, flushed, then renamed over the sidecar.

use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::ops::Range;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;

use roxmltree::{Document, Node, ParsingOptions};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::model::Edits;
use crate::store::Store;

/// The biggest sidecar read (a bigger one is refused, never cut short).
pub const MAX_SIDECAR: u64 = 1024 * 1024;
/// The most XML nodes parsed in one sidecar.
pub const MAX_NODES: u32 = 100_000;
/// The longest `starbase:Edits` JSON read.
const MAX_EDITS_JSON: usize = 16 * 1024;

pub const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
pub const CRS: &str = "http://ns.adobe.com/camera-raw-settings/1.0/";
pub const STARBASE: &str = "https://github.com/DomeShifter/Starbase/ns/photo-edit/1.0/";
const DARKTABLE: &str = "http://darktable.sf.net/";
const XML: &str = "http://www.w3.org/XML/1998/namespace";
/// The Camera Raw process version written with our `crs:` fields (the "2012" tone controls).
pub const PROCESS_VERSION: &str = "11.0";

/// The `crs:` fields we own: written from our edits, removed on reset.
const OWNED_CRS: [&str; 14] = [
    "HasCrop",
    "CropTop",
    "CropLeft",
    "CropBottom",
    "CropRight",
    "CropAngle",
    "Exposure2012",
    "Contrast2012",
    "Highlights2012",
    "Shadows2012",
    "Saturation",
    "Vibrance",
    "IncrementalTemperature",
    "IncrementalTint",
];
/// `crs:` bookkeeping fields that aren't edits (they don't make a sidecar "Lightroom's").
const CRS_BOOKKEEPING: [&str; 7] =
    ["ProcessVersion", "Version", "CompatibleVersion", "HasSettings", "AlreadyApplied", "RawFileName", "CropConstrainToWarp"];

/// Where the edits read from a sidecar came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// No edits of ours or Adobe's in it (or no sidecar).
    Nothing,
    /// `starbase:Edits`.
    Starbase,
    /// Only `crs:` fields: imported approximately.
    Crs,
}

/// A sidecar read.
#[derive(Clone, Debug)]
pub struct Parsed {
    pub edits: Edits,
    pub source: Source,
    /// Other editors whose own edits are in it: "darktable", "lightroom".
    pub foreign: Vec<&'static str>,
}

fn options() -> ParsingOptions {
    ParsingOptions { allow_dtd: false, nodes_limit: MAX_NODES }
}

/// The sidecar's text: UTF-8, without a byte-order mark (returned separately, kept on writing).
fn text_of(bytes: &[u8]) -> Result<(&str, &str), String> {
    let s = std::str::from_utf8(bytes).map_err(|_| "it isn't UTF-8 text".to_string())?;
    Ok(match s.strip_prefix('\u{FEFF}') {
        Some(rest) => ("\u{FEFF}", rest),
        None => ("", s),
    })
}

fn parse(text: &str) -> Result<Document<'_>, String> {
    Document::parse_with_options(text, options()).map_err(|e| format!("it isn't valid XML ({e})"))
}

fn rdf_root<'a, 'i>(doc: &'a Document<'i>) -> Option<Node<'a, 'i>> {
    doc.descendants().find(|n| n.is_element() && n.tag_name().namespace() == Some(RDF) && n.tag_name().name() == "RDF")
}

/// The `rdf:Description`s directly in `rdf:RDF`.
fn descriptions<'a, 'i>(rdf: Node<'a, 'i>) -> impl Iterator<Item = Node<'a, 'i>> {
    rdf.children().filter(|n| n.is_element() && n.tag_name().namespace() == Some(RDF) && n.tag_name().name() == "Description")
}

/// One property of a description: an attribute or a child element.
struct Prop<'i> {
    ns: Option<&'i str>,
    name: &'i str,
    range: Range<usize>,
    /// The simple value (an attribute, or an element holding only text).
    value: Option<String>,
}

fn props<'a>(d: Node<'a, '_>) -> Vec<Prop<'a>> {
    let mut out = Vec::new();
    for a in d.attributes() {
        if matches!(a.namespace(), Some(RDF) | Some(XML)) {
            continue; // rdf:about, rdf:ID, xml:lang: not properties
        }
        out.push(Prop { ns: a.namespace(), name: a.name(), range: a.range(), value: Some(a.value().to_string()) });
    }
    for c in d.children().filter(Node::is_element) {
        let simple = c.children().all(|n| n.is_text());
        out.push(Prop {
            ns: c.tag_name().namespace(),
            name: c.tag_name().name(),
            range: c.range(),
            value: simple.then(|| c.children().filter_map(|n| n.text()).collect()),
        });
    }
    out
}

fn is_ours(ns: Option<&str>, name: &str, our_description: bool) -> bool {
    ns == Some(STARBASE) || (ns == Some(CRS) && (OWNED_CRS.contains(&name) || (our_description && name == "ProcessVersion")))
}

/// Reads a sidecar's edits (identity when it holds none).
pub fn read(bytes: &[u8]) -> Result<Parsed, String> {
    let (_, text) = text_of(bytes)?;
    let doc = parse(text)?;
    let rdf = rdf_root(&doc).ok_or("it has no rdf:RDF")?;
    let mut foreign = Vec::new();
    // darktable: a history with at least one step.
    let dt_history = doc.descendants().any(|n| {
        n.is_element()
            && n.tag_name().namespace() == Some(DARKTABLE)
            && n.tag_name().name() == "history"
            && n.descendants().any(|li| li.is_element() && li.tag_name().namespace() == Some(RDF) && li.tag_name().name() == "li")
    });
    if dt_history {
        foreign.push("darktable");
    }
    let mut ours: Option<String> = None;
    let mut crs: Vec<(String, String)> = Vec::new();
    let mut other_crs = false;
    for d in descriptions(rdf) {
        for p in props(d) {
            match p.ns {
                Some(STARBASE) if p.name == "Edits" => {
                    if ours.is_none() {
                        ours = p.value;
                    }
                }
                Some(CRS) if OWNED_CRS.contains(&p.name) => {
                    if let Some(v) = p.value {
                        crs.push((p.name.to_string(), v));
                    }
                }
                Some(CRS) if !CRS_BOOKKEEPING.contains(&p.name) => other_crs = true,
                _ => {}
            }
        }
    }
    let from_ours = ours.filter(|j| j.len() <= MAX_EDITS_JSON).and_then(|j| serde_json::from_str::<Edits>(&j).ok());
    if other_crs || (from_ours.is_none() && !crs.is_empty()) {
        foreign.push("lightroom");
    }
    let (edits, source) = match from_ours {
        Some(e) => (e, Source::Starbase),
        None if !crs.is_empty() => (import_crs(&crs), Source::Crs),
        None => (Edits::default(), Source::Nothing),
    };
    Ok(Parsed { edits, source, foreign })
}

/// Our edits from Adobe's `crs:` fields, approximately (values clamped into our ranges).
fn import_crs(crs: &[(String, String)]) -> Edits {
    let get =
        |k: &str| crs.iter().find(|(n, _)| n == k).and_then(|(_, v)| v.trim().trim_start_matches('+').parse::<f64>().ok()).filter(|v| v.is_finite());
    let int = |k: &str| get(k).map_or(0, |v| v.round().clamp(-100.0, 100.0) as i64);
    let mut e = serde_json::json!({
        "exposure": get("Exposure2012").map_or(0.0, |v| v.clamp(-5.0, 5.0)),
        "contrast": int("Contrast2012"),
        "highlights": int("Highlights2012"),
        "shadows": int("Shadows2012"),
        "saturation": int("Saturation"),
        "vibrance": int("Vibrance"),
        "temperature": int("IncrementalTemperature"),
        "tint": int("IncrementalTint"),
    });
    let has_crop = crs.iter().any(|(n, v)| n == "HasCrop" && v.trim().eq_ignore_ascii_case("true"));
    if has_crop {
        let f = |k: &str, d: f64| get(k).map_or(d, |v| v.clamp(0.0, 1.0));
        let (l, t, r, b) = (f("CropLeft", 0.0), f("CropTop", 0.0), f("CropRight", 1.0), f("CropBottom", 1.0));
        if r - l >= 0.01 && b - t >= 0.01 {
            e["crop"] = serde_json::json!({"l": l, "t": t, "r": r, "b": b});
        }
        e["angle"] = serde_json::json!(get("CropAngle").map_or(0.0, |v| v.clamp(-45.0, 45.0)));
    }
    serde_json::from_value(e).unwrap_or_default()
}

/// Text safe inside a double-quoted XML attribute.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            c => out.push(c),
        }
    }
    out
}

/// The `crs:` fields for these edits (only what differs from no change), then ours.
fn fields(e: &Edits) -> Vec<(String, String)> {
    let j = serde_json::to_value(e).unwrap_or(Value::Null);
    let num = |k: &str| j.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    let mut out: Vec<(String, String)> = vec![("crs:ProcessVersion".into(), PROCESS_VERSION.into())];
    let crop = j.get("crop");
    let c = |k: &str, d: f64| crop.and_then(|c| c.get(k)).and_then(Value::as_f64).unwrap_or(d);
    let (l, t, r, b, angle) = (c("l", 0.0), c("t", 0.0), c("r", 1.0), c("b", 1.0), num("angle"));
    if (l, t, r, b) != (0.0, 0.0, 1.0, 1.0) || angle != 0.0 {
        out.push(("crs:HasCrop".into(), "True".into()));
        for (k, v) in [("CropTop", t), ("CropLeft", l), ("CropBottom", b), ("CropRight", r)] {
            out.push((format!("crs:{k}"), format!("{v:.6}")));
        }
        out.push(("crs:CropAngle".into(), format!("{angle:.4}")));
    }
    if num("exposure") != 0.0 {
        out.push(("crs:Exposure2012".into(), format!("{:+.2}", num("exposure"))));
    }
    for (k, crs) in [
        ("contrast", "Contrast2012"),
        ("highlights", "Highlights2012"),
        ("shadows", "Shadows2012"),
        ("saturation", "Saturation"),
        ("vibrance", "Vibrance"),
        ("temperature", "IncrementalTemperature"),
        ("tint", "IncrementalTint"),
    ] {
        let v = num(k).round() as i64;
        if v != 0 {
            out.push((format!("crs:{crs}"), format!("{v:+}")));
        }
    }
    out.push(("starbase:EditsVersion".into(), "1".into()));
    out.push(("starbase:Edits".into(), serde_json::to_string(e).unwrap_or_default()));
    out
}

/// Our `rdf:Description`, declaring every namespace it uses itself.
fn our_description(e: &Edits) -> String {
    let mut s = format!("\n  <rdf:Description rdf:about=\"\"\n    xmlns:rdf=\"{RDF}\"\n    xmlns:crs=\"{CRS}\"\n    xmlns:starbase=\"{STARBASE}\"");
    for (k, v) in fields(e) {
        s.push_str(&format!("\n   {k}=\"{}\"", escape(&v)));
    }
    s.push_str("/>");
    s
}

/// A new sidecar holding only `e`.
fn fresh(e: &Edits) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"Starbase\">\n <rdf:RDF xmlns:rdf=\"{RDF}\">{}\n </rdf:RDF>\n</x:xmpmeta>\n",
        our_description(e)
    )
}

/// Back from `at` over spaces, tabs and line breaks.
fn back_over_space(text: &str, at: usize) -> usize {
    at - text[..at].bytes().rev().take_while(|b| b.is_ascii_whitespace()).count()
}

/// The result of merging edits into a sidecar.
#[derive(Debug, PartialEq, Eq)]
pub enum Merged {
    /// Write this.
    Write(Vec<u8>),
    /// Nothing of anyone's is left: delete the sidecar.
    Delete,
    /// No change to make (no sidecar and no edits, or a reset of a sidecar without ours).
    Same,
}

/// Merges `edits` (None or identity = take ours out) into the sidecar `existing` (None = there
/// is none). Everything not ours is kept byte for byte.
pub fn merge(existing: Option<&[u8]>, edits: Option<&Edits>) -> Result<Merged, String> {
    let edits = edits.filter(|e| !e.is_identity());
    let Some(existing) = existing else {
        return Ok(match edits {
            Some(e) => Merged::Write(checked(fresh(e).into_bytes(), Some(e))?),
            None => Merged::Same,
        });
    };
    let (bom, text) = text_of(existing)?;
    let doc = parse(text)?;
    let rdf = rdf_root(&doc).ok_or("it has no rdf:RDF")?;
    // (start, end, replacement), applied from the end.
    let mut cuts: Vec<(usize, usize, String)> = Vec::new();
    for d in descriptions(rdf) {
        let ps = props(d);
        let mine = ps.iter().any(|p| p.ns == Some(STARBASE));
        let owned: Vec<&Prop> = ps.iter().filter(|p| is_ours(p.ns, p.name, mine)).collect();
        if owned.is_empty() {
            continue;
        }
        if owned.len() == ps.len() {
            let r = d.range();
            cuts.push((back_over_space(text, r.start), r.end, String::new()));
        } else {
            for p in owned {
                cuts.push((back_over_space(text, p.range.start), p.range.end, String::new()));
            }
        }
    }
    let removed = !cuts.is_empty();
    if let Some(e) = edits {
        let r = rdf.range();
        if text[r.clone()].ends_with("/>") {
            // <rdf:RDF …/>: opened and closed around ours.
            let qname: String = text[r.start + 1..].chars().take_while(|c| !c.is_whitespace() && *c != '/' && *c != '>').collect();
            cuts.push((r.end - 2, r.end, format!(">{}\n </{qname}>", our_description(e))));
        } else {
            let close = text[..r.end].rfind("</").ok_or("its rdf:RDF has no end")?;
            let at = back_over_space(text, close).max(r.start);
            cuts.push((at, at, our_description(e)));
        }
    } else if !removed {
        return Ok(Merged::Same);
    }
    cuts.sort_by_key(|c| (c.0, c.1));
    let mut out = String::with_capacity(text.len() + 2048);
    out.push_str(bom);
    let mut at = 0;
    for (start, end, with) in &cuts {
        if *start < at {
            return Err("overlapping changes".into()); // can't happen: descriptions don't nest
        }
        out.push_str(&text[at..*start]);
        out.push_str(with);
        at = *end;
    }
    out.push_str(&text[at..]);
    if edits.is_none() && empty(&out)? {
        return Ok(Merged::Delete);
    }
    Ok(Merged::Write(checked(out.into_bytes(), edits)?))
}

/// Whether a sidecar holds no property of anyone's.
fn empty(text: &str) -> Result<bool, String> {
    let (_, text) = text_of(text.as_bytes())?;
    let doc = parse(text)?;
    let rdf = rdf_root(&doc).ok_or("it has no rdf:RDF")?;
    Ok(descriptions(rdf).all(|d| props(d).is_empty()) && rdf.children().filter(Node::is_element).count() == descriptions(rdf).count())
}

/// The merged sidecar must parse and read back as exactly the edits written (or none).
fn checked(bytes: Vec<u8>, edits: Option<&Edits>) -> Result<Vec<u8>, String> {
    if bytes.len() as u64 > MAX_SIDECAR {
        return Err("the sidecar would be too big".into());
    }
    let back = read(&bytes)?;
    let want = edits.cloned().unwrap_or_default();
    let ok = match edits {
        Some(_) => back.source == Source::Starbase && back.edits == want,
        None => back.source != Source::Starbase,
    };
    if !ok {
        return Err("the merged sidecar didn't read back as written".into());
    }
    Ok(bytes)
}

/// A sidecar's version tag (the `etag` of the edits API): "none" when there is no sidecar.
pub fn etag(bytes: Option<&[u8]>) -> String {
    match bytes {
        Some(b) => hex::encode(&Sha256::digest(b)[..8]),
        None => "none".into(),
    }
}

/// What thumbnails depend on: a hash of the edits (None for no change).
pub fn edits_hash(e: &Edits) -> Option<String> {
    if e.is_identity() {
        return None;
    }
    Some(hex::encode(&Sha256::digest(serde_json::to_string(e).unwrap_or_default().as_bytes())[..8]))
}

/// The thumbnail cache key of a picture with this content and these edits.
pub fn thumb_key(content: &str, edits: Option<&str>) -> String {
    match edits {
        None => content.to_string(),
        Some(e) => hex::encode(Sha256::digest(format!("{content}:{e}").as_bytes())),
    }
}

/// A file's size and modified time, as kept in the index to notice a changed sidecar.
pub fn stat_of(m: &std::fs::Metadata) -> String {
    format!("{}-{}", m.len(), m.mtime().saturating_mul(1_000_000_000).saturating_add(m.mtime_nsec()))
}

// ------------------------------------------------------------------ on disk

/// Why a sidecar can't be used.
#[derive(Debug, PartialEq, Eq)]
pub enum SideError {
    /// A symbolic link, or not a plain file.
    NotAFile,
    TooBig,
    Io(String),
}

impl SideError {
    pub fn say(&self, name: &str) -> String {
        match self {
            SideError::NotAFile => format!("Its sidecar ({name}) is a link or not a plain file, so Starbase won't read or change it."),
            SideError::TooBig => format!("Its sidecar ({name}) is too big (over 1 MB) to read."),
            SideError::Io(e) => format!("Its sidecar ({name}) can't be read: {e}"),
        }
    }
}

/// A sidecar read from disk: its bytes and size-and-time stat.
pub struct OnDisk {
    pub bytes: Vec<u8>,
    pub stat: String,
}

/// A picture's folder, opened without following links and checked to be the folder asked for.
pub struct Folder {
    fd: OwnedFd,
}

fn cname(name: &str) -> io::Result<CString> {
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "bad name"));
    }
    CString::new(name).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "bad name"))
}

fn check(r: libc::c_int) -> io::Result<libc::c_int> {
    if r < 0 { Err(io::Error::last_os_error()) } else { Ok(r) }
}

impl Folder {
    /// The folder `folder` of `store` ("" = the top), if it is a real folder reached without links.
    pub fn open(store: &Store, folder: &str) -> Option<Folder> {
        use std::os::unix::ffi::OsStrExt;
        if !store.no_links(folder) {
            return None;
        }
        let full = store.full(folder);
        let c = CString::new(full.as_os_str().as_bytes()).ok()?;
        // SAFETY: a valid C string; the returned descriptor is owned below.
        let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
        if fd < 0 {
            return None;
        }
        // SAFETY: fd was just opened and is owned by nobody else.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        // The folder opened is the one checked (no link swapped into the path in between).
        match std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd())) {
            Ok(real) => {
                let want = std::fs::canonicalize(&store.root).ok()?.join(folder);
                if real != want.components().collect::<std::path::PathBuf>() {
                    return None;
                }
            }
            Err(_) => {
                let a = File::from(fd.try_clone().ok()?).metadata().ok()?;
                let b = std::fs::symlink_metadata(&full).ok()?;
                if (a.dev(), a.ino()) != (b.dev(), b.ino()) || !store.no_links(folder) {
                    return None;
                }
            }
        }
        Some(Folder { fd })
    }

    /// Why nothing can be written here, if so: a read-only file system, or no write permission.
    pub fn read_only(&self) -> Option<&'static str> {
        // SAFETY: fstatvfs fills the struct we give it, from our own open descriptor.
        let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstatvfs(self.fd.as_raw_fd(), &mut s) } == 0 && s.f_flag & libc::ST_RDONLY != 0 {
            return Some("This folder is on read-only storage, so edits can't be saved here.");
        }
        let dot = c".";
        // SAFETY: our own descriptor and a constant C string.
        if unsafe { libc::faccessat(self.fd.as_raw_fd(), dot.as_ptr(), libc::W_OK, libc::AT_EACCESS) } != 0 {
            return Some("The server isn't allowed to write in this folder, so edits can't be saved here.");
        }
        None
    }

    /// Reads the sidecar `name` in this folder: None when there is none.
    pub fn read(&self, name: &str) -> Result<Option<OnDisk>, SideError> {
        let c = cname(name).map_err(|e| SideError::Io(e.to_string()))?;
        // SAFETY: our own descriptor and a valid C string; the result is owned below.
        let fd = unsafe { libc::openat(self.fd.as_raw_fd(), c.as_ptr(), libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC) };
        if fd < 0 {
            let e = io::Error::last_os_error();
            return match e.raw_os_error() {
                Some(libc::ENOENT) => Ok(None),
                Some(libc::ELOOP) | Some(libc::ENXIO) | Some(libc::EISDIR) => Err(SideError::NotAFile),
                _ => Err(SideError::Io(e.to_string())),
            };
        }
        // SAFETY: fd was just opened and is owned by nobody else.
        let f = unsafe { File::from_raw_fd(fd) };
        let m = f.metadata().map_err(|e| SideError::Io(e.to_string()))?;
        if !m.is_file() {
            return Err(SideError::NotAFile);
        }
        if m.len() > MAX_SIDECAR {
            return Err(SideError::TooBig);
        }
        let mut bytes = Vec::with_capacity(m.len() as usize);
        f.take(MAX_SIDECAR + 1).read_to_end(&mut bytes).map_err(|e| SideError::Io(e.to_string()))?;
        if bytes.len() as u64 > MAX_SIDECAR {
            return Err(SideError::TooBig);
        }
        Ok(Some(OnDisk { bytes, stat: stat_of(&m) }))
    }

    /// Replaces (or makes) `name` with `bytes`: a hidden temp file in this folder, flushed, then
    /// renamed over it (a link there is replaced, never followed). Returns the new stat.
    pub fn write(&self, name: &str, bytes: &[u8]) -> io::Result<String> {
        let target = cname(name)?;
        let tmp_name = format!(".{name}.{}.tmp", crate::store::random_hex(6));
        let tmp = cname(&tmp_name)?;
        let dir = self.fd.as_raw_fd();
        // SAFETY: our own descriptor and valid C strings; the result is owned below.
        let fd = check(unsafe {
            libc::openat(dir, tmp.as_ptr(), libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC, 0o664)
        })?;
        // SAFETY: fd was just opened and is owned by nobody else.
        let mut f = unsafe { File::from_raw_fd(fd) };
        let done = f.write_all(bytes).and_then(|()| f.sync_all()).and_then(|()| f.metadata());
        let m = match done {
            Ok(m) => m,
            Err(e) => {
                // SAFETY: as above.
                unsafe { libc::unlinkat(dir, tmp.as_ptr(), 0) };
                return Err(e);
            }
        };
        drop(f);
        // SAFETY: as above.
        if let Err(e) = check(unsafe { libc::renameat(dir, tmp.as_ptr(), dir, target.as_ptr()) }) {
            // SAFETY: as above.
            unsafe { libc::unlinkat(dir, tmp.as_ptr(), 0) };
            return Err(e);
        }
        Ok(stat_of(&m))
    }

    /// Deletes the sidecar `name` (a link there is removed, never followed).
    pub fn remove(&self, name: &str) -> io::Result<()> {
        let c = cname(name)?;
        // SAFETY: our own descriptor and a valid C string.
        match check(unsafe { libc::unlinkat(self.fd.as_raw_fd(), c.as_ptr(), 0) }) {
            Err(e) if e.raw_os_error() == Some(libc::ENOENT) => Ok(()),
            r => r.map(|_| ()),
        }
    }
}

/// The sidecar's name for a picture named `name`.
pub fn name_for(picture: &str) -> String {
    format!("{picture}.xmp")
}

#[cfg(test)]
#[path = "sidecar_tests.rs"]
mod tests;
