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
//!   expansion: roxmltree refuses them), at most [`MAX_NODES`] nodes, UTF-8 only, and attributes
//!   counted before parsing ([`MAX_ATTRS_PER_ELEMENT`], [`MAX_ATTRS`]): roxmltree compares each
//!   attribute with the element's others, so 100 000 attributes on one element took seconds.
//!   The same pass caps nesting ([`MAX_DEPTH`]: roxmltree recurses per level, and deep nesting
//!   overflowed the stack and aborted the server) and namespace declarations ([`MAX_NS`]). The
//!   edits API parses outside the server's store lock, so a slow sidecar can't stall other work.
//! - **On disk**: the folder is opened once without following links and checked to be the
//!   folder asked for; the sidecar is read, written and removed relative to that open folder
//!   (`openat`, `renameat`), never through a symbolic link: a sidecar that is a link is refused.
//!   Writes go to a hidden temp file in the same folder (with the old sidecar's permissions),
//!   flushed; the sidecar is checked to be unchanged since it was read (another app may have
//!   saved), then the temp file is renamed over it and the folder flushed.

use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::ops::Range;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;

use roxmltree::{Document, Node, ParsingOptions};
use sha2::{Digest, Sha256};

use super::crs;
pub use super::crs::Geo;
use super::model::Edits;
use crate::store::Store;
use starbase_photo_edit::{Filter, FilterName};

/// The biggest sidecar read (a bigger one is refused, never cut short). darktable's sidecars with
/// long histories are tens of KB.
pub const MAX_SIDECAR: u64 = 256 * 1024;
/// The most attributes on one element, and in a whole sidecar (checked before parsing).
pub const MAX_ATTRS_PER_ELEMENT: usize = 256;
pub const MAX_ATTRS: usize = 20_000;
/// The most XML nodes parsed in one sidecar.
pub const MAX_NODES: u32 = 100_000;
/// Elements open at once: XMP needs about 8.
pub const MAX_DEPTH: usize = 64;
/// Namespace declarations (`xmlns`, `xmlns:x`) on one element, and in the whole sidecar.
pub const MAX_NS_PER_ELEMENT: usize = 32;
pub const MAX_NS: usize = 256;
/// The longest `starbase:Edits` JSON read.
const MAX_EDITS_JSON: usize = 16 * 1024;

pub const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
pub const CRS: &str = "http://ns.adobe.com/camera-raw-settings/1.0/";
pub const STARBASE: &str = "https://github.com/DomeShifter/Starbase/ns/photo-edit/1.0/";
pub const TIFF: &str = "http://ns.adobe.com/tiff/1.0/";
/// The `tiff:` fields we write (the picture's stored size and final orientation: docs/photos-edit.md 2.1).
const OWNED_TIFF: [&str; 3] = ["ImageWidth", "ImageLength", "Orientation"];
/// darktable's automatic history steps for a JPEG it merely imported (not someone's edits).
const DARKTABLE_DEFAULTS: [&str; 3] = ["colorin", "colorout", "gamma"];
const DARKTABLE: &str = "http://darktable.sf.net/";
const XML: &str = "http://www.w3.org/XML/1998/namespace";
/// The Camera Raw process version written with our `crs:` fields (the "2012" tone controls).
pub const PROCESS_VERSION: &str = "11.0";

/// The `crs:` fields we own: written from our edits, removed on reset.
const OWNED_CRS: [&str; 15] = [
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
    "ConvertToGrayscale",
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
    /// Other editors whose own edits are in it: "darktable" (history beyond its automatic
    /// steps), "crs" (Adobe's fields we don't write, or crs: edits without ours).
    pub foreign: Vec<&'static str>,
    /// `starbase:Edits` written by a newer Starbase (another `EditsVersion`): not ours to change.
    pub newer: bool,
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
    if let Some(why) = prescan(text.as_bytes()) {
        return Err(why.into());
    }
    Document::parse_with_options(text, options()).map_err(|e| format!("it isn't valid XML ({e})"))
}

/// What [`prescan`] found wrong, if anything: the shapes roxmltree handles badly are refused
/// before it sees them. Its parser recurses once per nesting level (5 000 levels overflowed a
/// 2 MB thread stack and aborted the server), and copies the namespaces in scope at every element
/// that declares one (9 000 nested declarations took minutes).
///
/// One pass over the bytes, no recursion: in tags (comments, CDATA and processing instructions
/// skipped) it counts `=` outside quotes as attributes ([`MAX_ATTRS_PER_ELEMENT`], [`MAX_ATTRS`]),
/// `xmlns` names as namespace declarations ([`MAX_NS_PER_ELEMENT`], [`MAX_NS`]), and open
/// elements ([`MAX_DEPTH`]). A DTD is refused (roxmltree would too). Counting too many only
/// refuses odd XML, never lets too much through.
fn prescan(b: &[u8]) -> Option<&'static str> {
    let after = |from: usize, end: &[u8]| b[from..].windows(end.len()).position(|w| w == end).map_or(b.len(), |p| from + p + end.len());
    let (mut i, mut total, mut ns_total, mut depth) = (0, 0usize, 0usize, 0usize);
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &b[i..];
        if rest.starts_with(b"<!--") {
            i = after(i + 4, b"-->");
            continue;
        }
        if rest.starts_with(b"<![CDATA[") {
            i = after(i + 9, b"]]>");
            continue;
        }
        if rest.starts_with(b"<?") {
            i = after(i + 2, b"?>");
            continue;
        }
        if rest.starts_with(b"<!") {
            return Some("it has a DTD");
        }
        let closing = rest.starts_with(b"</");
        let (mut here, mut ns_here, mut quote, mut last) = (0usize, 0usize, 0u8, 0u8);
        i += 1;
        while i < b.len() {
            let c = b[i];
            if quote != 0 {
                if c == quote {
                    quote = 0;
                }
            } else if c == b'"' || c == b'\'' {
                quote = c;
            } else if c == b'=' {
                here += 1;
            } else if c == b'>' {
                break;
            } else if c.is_ascii_whitespace() && b[i + 1..].starts_with(b"xmlns") {
                ns_here += 1;
            }
            if !c.is_ascii_whitespace() {
                last = c;
            }
            i += 1;
        }
        total += here;
        ns_total += ns_here;
        if here > MAX_ATTRS_PER_ELEMENT || total > MAX_ATTRS {
            return Some("it has too many attributes");
        }
        if ns_here > MAX_NS_PER_ELEMENT || ns_total > MAX_NS {
            return Some("it declares too many namespaces");
        }
        if closing {
            depth = depth.saturating_sub(1);
        } else if last != b'/' {
            depth += 1;
            if depth > MAX_DEPTH {
                return Some("it is nested too deeply");
            }
        }
    }
    None
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

/// Whether a property is ours to cut: our fields, and `crs:ProcessVersion` plus the `tiff:`
/// size and orientation in our own description or wherever they are when we write ours (which
/// carries its own), so none is ever there twice.
fn is_ours(ns: Option<&str>, name: &str, ours_or_writing: bool) -> bool {
    ns == Some(STARBASE)
        || (ns == Some(CRS) && (OWNED_CRS.contains(&name) || (ours_or_writing && name == "ProcessVersion")))
        || (ns == Some(TIFF) && ours_or_writing && OWNED_TIFF.contains(&name))
}

/// Whether darktable's history has a step someone made: anything but its automatic ones for a
/// JPEG (colorin, colorout, gamma, and flip named "auto"), which it adds on mere import.
fn darktable_edited(doc: &Document) -> bool {
    let attr = |n: &Node, name: &str| -> Option<String> {
        n.attributes().find(|a| a.namespace() == Some(DARKTABLE) && a.name() == name).map(|a| a.value().to_string()).or_else(|| {
            n.children()
                .find(|c| c.is_element() && c.tag_name().namespace() == Some(DARKTABLE) && c.tag_name().name() == name)
                .map(|c| c.children().filter_map(|t| t.text()).collect())
        })
    };
    doc.descendants().filter(|n| n.is_element() && n.tag_name().namespace() == Some(DARKTABLE) && n.tag_name().name() == "history").any(|h| {
        h.descendants().filter(|li| li.is_element() && li.tag_name().namespace() == Some(RDF) && li.tag_name().name() == "li").any(|li| {
            let op = attr(&li, "operation").unwrap_or_default();
            !(DARKTABLE_DEFAULTS.contains(&op.as_str()) || (op == "flip" && attr(&li, "multi_name").as_deref() == Some("auto")))
        })
    })
}

/// Reads a sidecar's edits (identity when it holds none), without knowing the picture: a crop
/// in Adobe's fields alone isn't imported. See [`read_for`].
pub fn read(bytes: &[u8]) -> Result<Parsed, String> {
    read_for(bytes, None)
}

/// Reads a sidecar's edits for the picture `geo` (its stored size and EXIF orientation, which
/// Adobe's crop and orientation fields are relative to).
pub fn read_for(bytes: &[u8], geo: Option<&Geo>) -> Result<Parsed, String> {
    let (_, text) = text_of(bytes)?;
    let doc = parse(text)?;
    let rdf = rdf_root(&doc).ok_or("it has no rdf:RDF")?;
    let mut foreign = Vec::new();
    if darktable_edited(&doc) {
        foreign.push("darktable");
    }
    let mut ours: Option<String> = None;
    let mut version: Option<String> = None;
    let mut crs: Vec<(String, String)> = Vec::new();
    let mut orientation: Option<u16> = None;
    let mut other_crs = false;
    for d in descriptions(rdf) {
        for p in props(d) {
            match p.ns {
                Some(STARBASE) if p.name == "Edits" => {
                    if ours.is_none() {
                        ours = p.value;
                    }
                }
                Some(STARBASE) if p.name == "EditsVersion" => version = version.or(p.value),
                Some(CRS) if OWNED_CRS.contains(&p.name) => {
                    if let Some(v) = p.value {
                        crs.push((p.name.to_string(), v));
                    }
                }
                Some(CRS) if !CRS_BOOKKEEPING.contains(&p.name) => other_crs = true,
                Some(TIFF) if p.name == "Orientation" => {
                    orientation = orientation.or(p.value.and_then(|v| v.trim().parse().ok()).filter(|o| (1..=8).contains(o)))
                }
                _ => {}
            }
        }
    }
    let newer = version.as_deref().is_some_and(|v| v.trim() != "1");
    let from_ours = ours.filter(|j| j.len() <= MAX_EDITS_JSON && !newer).and_then(|j| serde_json::from_str::<Edits>(&j).ok());
    let has_crs = !crs.is_empty() || orientation.is_some_and(|o| geo.is_some_and(|g| o != g.orientation));
    if other_crs || (from_ours.is_none() && !crs.is_empty()) {
        foreign.push("crs");
    }
    let (edits, source) = match from_ours {
        Some(e) => (e, Source::Starbase),
        None if has_crs => (import_crs(&crs, orientation, geo), Source::Crs),
        None => (Edits::default(), Source::Nothing),
    };
    Ok(Parsed { edits, source, foreign, newer })
}

/// Our edits from Adobe's `crs:` (and `tiff:Orientation`) fields, approximately (values
/// clamped into our ranges). The crop and turns only when the picture is known.
fn import_crs(crs: &[(String, String)], orientation: Option<u16>, geo: Option<&Geo>) -> Edits {
    let get =
        |k: &str| crs.iter().find(|(n, _)| n == k).and_then(|(_, v)| v.trim().trim_start_matches('+').parse::<f64>().ok()).filter(|v| v.is_finite());
    let int = |k: &str| get(k).map_or(0, |v| v.round().clamp(-100.0, 100.0) as i64);
    let e = serde_json::json!({
        "exposure": get("Exposure2012").map_or(0.0, |v| v.clamp(-5.0, 5.0)),
        "contrast": int("Contrast2012"),
        "highlights": int("Highlights2012"),
        "shadows": int("Shadows2012"),
        "saturation": int("Saturation"),
        "vibrance": int("Vibrance"),
        "temperature": int("IncrementalTemperature"),
        "tint": int("IncrementalTint"),
    });
    let mut e: Edits = serde_json::from_value(e).unwrap_or_default();
    let yes = |k: &str| crs.iter().any(|(n, v)| n == k && v.trim().eq_ignore_ascii_case("true"));
    if yes("ConvertToGrayscale") {
        e.filter = Some(Filter { name: FilterName::Bw, amount: 1.0 });
    }
    if let Some(g) = geo {
        let crop = yes("HasCrop").then(|| {
            let f = |k: &str, d: f64| get(k).map_or(d, |v| v.clamp(-1.0, 2.0));
            [f("CropLeft", 0.0), f("CropTop", 0.0), f("CropRight", 1.0), f("CropBottom", 1.0), get("CropAngle").unwrap_or(0.0)]
        });
        let (rot90, flip_h, angle, crop) = crs::from_crs(crop, orientation.unwrap_or(g.orientation), g);
        (e.rot90, e.flip_h, e.angle, e.crop) = (rot90, flip_h, angle, crop);
    }
    e
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

/// The `tiff:` and `crs:` fields for these edits on picture `g` (only tone fields that differ
/// from no change), then ours. Numbers as plain decimals, never rationals (darktable ignores them).
fn fields(e: &Edits, g: &Geo) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![
        ("tiff:ImageWidth".into(), g.width.to_string()),
        ("tiff:ImageLength".into(), g.height.to_string()),
        ("tiff:Orientation".into(), crs::final_orientation(g.orientation, e.rot90, e.flip_h).to_string()),
        ("crs:ProcessVersion".into(), PROCESS_VERSION.into()),
    ];
    match crs::crop_to_crs(e, g) {
        Some([l, t, r, b, angle]) => {
            out.push(("crs:HasCrop".into(), "True".into()));
            for (k, v) in [("CropLeft", l), ("CropTop", t), ("CropRight", r), ("CropBottom", b), ("CropAngle", angle)] {
                out.push((format!("crs:{k}"), format!("{v:.6}")));
            }
        }
        None => out.push(("crs:HasCrop".into(), "False".into())),
    }
    if e.exposure != 0.0 {
        out.push(("crs:Exposure2012".into(), format!("{:+.2}", e.exposure)));
    }
    for (v, crs) in [
        (e.contrast, "Contrast2012"),
        (e.highlights, "Highlights2012"),
        (e.shadows, "Shadows2012"),
        (e.saturation, "Saturation"),
        (e.vibrance, "Vibrance"),
        (e.temperature, "IncrementalTemperature"),
        (e.tint, "IncrementalTint"),
    ] {
        if v != 0 {
            out.push((format!("crs:{crs}"), format!("{v:+}")));
        }
    }
    if e.filter.is_some_and(|f| f.name == FilterName::Bw && f.amount >= 0.5) {
        out.push(("crs:ConvertToGrayscale".into(), "True".into()));
    }
    out.push(("starbase:EditsVersion".into(), "1".into()));
    out.push(("starbase:Edits".into(), serde_json::to_string(e).unwrap_or_default()));
    out
}

/// Our `rdf:Description`, declaring every namespace it uses itself.
fn our_description(e: &Edits, g: &Geo) -> String {
    let mut s = format!(
        "\n  <rdf:Description rdf:about=\"\"\n    xmlns:rdf=\"{RDF}\"\n    xmlns:tiff=\"{TIFF}\"\n    xmlns:crs=\"{CRS}\"\n    xmlns:starbase=\"{STARBASE}\""
    );
    for (k, v) in fields(e, g) {
        s.push_str(&format!("\n   {k}=\"{}\"", escape(&v)));
    }
    s.push_str("/>");
    s
}

/// A new sidecar holding only `e`.
fn fresh(e: &Edits, g: &Geo) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"Starbase\">\n <rdf:RDF xmlns:rdf=\"{RDF}\">{}\n </rdf:RDF>\n</x:xmpmeta>\n",
        our_description(e, g)
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

/// Merges `edits` (None or identity = take ours out) for picture `g` into the sidecar
/// `existing` (None = there is none). Everything not ours is kept byte for byte.
pub fn merge(existing: Option<&[u8]>, edits: Option<&Edits>, g: &Geo) -> Result<Merged, String> {
    let edits = edits.filter(|e| !e.is_identity());
    let Some(existing) = existing else {
        return Ok(match edits {
            Some(e) => Merged::Write(checked(fresh(e, g).into_bytes(), Some(e))?),
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
        let owned: Vec<&Prop> = ps.iter().filter(|p| is_ours(p.ns, p.name, mine || edits.is_some())).collect();
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
            cuts.push((r.end - 2, r.end, format!(">{}\n </{qname}>", our_description(e, g))));
        } else {
            let close = text[..r.end].rfind("</").ok_or("its rdf:RDF has no end")?;
            let at = back_over_space(text, close).max(r.start);
            cuts.push((at, at, our_description(e, g)));
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
            SideError::TooBig => format!("Its sidecar ({name}) is too big (over 256 KB) to read."),
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

    /// The size-and-time stat and permissions of `name` here, None when there is nothing there
    /// (a link is reported as itself, never followed).
    fn current(&self, name: &CString) -> io::Result<Option<(String, libc::mode_t)>> {
        // SAFETY: fstatat fills the struct we give it; our own descriptor and a valid C string.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstatat(self.fd.as_raw_fd(), name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) } != 0 {
            let e = io::Error::last_os_error();
            return if e.raw_os_error() == Some(libc::ENOENT) { Ok(None) } else { Err(e) };
        }
        let ns = (st.st_mtime as i64).saturating_mul(1_000_000_000).saturating_add(st.st_mtime_nsec as i64);
        Ok(Some((format!("{}-{ns}", st.st_size), st.st_mode & 0o7777)))
    }

    /// Replaces (or makes) `name` with `bytes`, if it is still as it was read (`expect`: its stat
    /// then, None = there was none): a hidden temp file in this folder, flushed, then renamed over
    /// it (a link there is replaced, never followed) and the folder flushed. Returns the new stat.
    pub fn write(&self, name: &str, bytes: &[u8], expect: Option<&str>) -> Result<String, WriteError> {
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
        let gone = || {
            // SAFETY: as above.
            unsafe { libc::unlinkat(dir, tmp.as_ptr(), 0) };
        };
        let done = (|| -> Result<std::fs::Metadata, WriteError> {
            let now = self.current(&target)?;
            if now.as_ref().map(|n| n.0.as_str()) != expect {
                return Err(WriteError::Changed);
            }
            if let Some((_, mode)) = now {
                // The old sidecar's permissions (not a link: a link's stat differs from any read).
                // SAFETY: our own open file.
                check(unsafe { libc::fchmod(f.as_raw_fd(), mode) })?;
            }
            f.write_all(bytes)?;
            f.sync_all()?;
            Ok(f.metadata()?)
        })();
        let m = match done {
            Ok(m) => m,
            Err(e) => {
                gone();
                return Err(e);
            }
        };
        drop(f);
        // Checked again right before the rename: another app's save in the meantime wins.
        if self.current(&target)?.map(|n| n.0).as_deref() != expect {
            gone();
            return Err(WriteError::Changed);
        }
        // SAFETY: as above.
        if let Err(e) = check(unsafe { libc::renameat(dir, tmp.as_ptr(), dir, target.as_ptr()) }) {
            gone();
            return Err(e.into());
        }
        // SAFETY: our own descriptor.
        unsafe { libc::fsync(dir) };
        Ok(stat_of(&m))
    }

    /// Deletes the sidecar `name` if it is still as it was read (`expect`); a link there is
    /// removed, never followed.
    pub fn remove(&self, name: &str, expect: &str) -> Result<(), WriteError> {
        let c = cname(name)?;
        match self.current(&c)? {
            None => return Ok(()),
            Some((now, _)) if now != expect => return Err(WriteError::Changed),
            Some(_) => {}
        }
        // SAFETY: our own descriptor and a valid C string.
        match check(unsafe { libc::unlinkat(self.fd.as_raw_fd(), c.as_ptr(), 0) }) {
            Err(e) if e.raw_os_error() != Some(libc::ENOENT) => return Err(e.into()),
            _ => {}
        }
        // SAFETY: our own descriptor.
        unsafe { libc::fsync(self.fd.as_raw_fd()) };
        Ok(())
    }
}

/// Why a sidecar wasn't written or removed.
#[derive(Debug)]
pub enum WriteError {
    /// It changed since it was read (another app saved): nothing was done.
    Changed,
    Io(io::Error),
}

impl From<io::Error> for WriteError {
    fn from(e: io::Error) -> Self {
        WriteError::Io(e)
    }
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::Changed => f.write_str("it was changed by someone else meanwhile"),
            WriteError::Io(e) => e.fmt(f),
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

// ---------------------------------------------------------------- the Info panel's fields

/// The namespace of the Info panel's fields (title, caption, labels, rights), kept in their own
/// `rdf:Description` so the edits code above never touches them and they never touch the edits.
pub const INFO: &str = "https://github.com/DomeShifter/Starbase/ns/photo-info/1.0/";

/// The longest text field and label, and the most labels.
pub const MAX_INFO_TEXT: usize = 2000;
pub const MAX_LABEL: usize = 60;
pub const MAX_LABELS: usize = 50;

/// What people type in the Info panel about a picture, saved in its sidecar, never in the
/// picture itself. Empty strings and lists = not set.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Info {
    pub title: String,
    pub caption: String,
    pub labels: Vec<String>,
    pub subject: String,
    pub copyright: String,
    pub artist: String,
    pub licence: String,
}

impl Info {
    pub fn is_empty(&self) -> bool {
        *self == Info::default()
    }

    /// Trimmed, labels without blanks or repeats; an error says what is too long.
    pub fn cleaned(mut self) -> Result<Info, String> {
        for (name, v) in [
            ("title", &mut self.title),
            ("caption", &mut self.caption),
            ("subject", &mut self.subject),
            ("copyright", &mut self.copyright),
            ("artist", &mut self.artist),
            ("licence", &mut self.licence),
        ] {
            *v = v.trim().to_string();
            if v.chars().count() > MAX_INFO_TEXT {
                return Err(format!("The {name} is too long ({MAX_INFO_TEXT} characters at most)."));
            }
        }
        let mut labels: Vec<String> = Vec::new();
        for l in self.labels.iter().map(|l| l.trim()).filter(|l| !l.is_empty()) {
            if l.chars().count() > MAX_LABEL {
                return Err(format!("A label is too long ({MAX_LABEL} characters at most)."));
            }
            if !labels.iter().any(|k| k.eq_ignore_ascii_case(l)) {
                labels.push(l.to_string());
            }
        }
        if labels.len() > MAX_LABELS {
            return Err(format!("Too many labels ({MAX_LABELS} at most)."));
        }
        self.labels = labels;
        Ok(self)
    }
}

/// The Info fields in a sidecar (none = nothing set, or not readable).
pub fn read_info(bytes: &[u8]) -> Info {
    let Ok((_, text)) = text_of(bytes) else { return Info::default() };
    let Ok(doc) = parse(text) else { return Info::default() };
    let Some(rdf) = rdf_root(&doc) else { return Info::default() };
    descriptions(rdf)
        .flat_map(props)
        .find(|p| p.ns == Some(INFO) && p.name == "Info")
        .and_then(|p| p.value)
        .filter(|j| j.len() <= MAX_SIDECAR as usize)
        .and_then(|j| serde_json::from_str::<Info>(&j).ok())
        .and_then(|i| i.cleaned().ok())
        .unwrap_or_default()
}

fn info_description(i: &Info) -> String {
    format!(
        "\n  <rdf:Description rdf:about=\"\"\n    xmlns:rdf=\"{RDF}\"\n    xmlns:starbaseinfo=\"{INFO}\"\n   starbaseinfo:InfoVersion=\"1\"\n   starbaseinfo:Info=\"{}\"/>",
        escape(&serde_json::to_string(i).unwrap_or_default())
    )
}

/// Merges the Info fields `info` (empty = take them out) into the sidecar `existing` (None =
/// none yet). Everything else, the edits included, is kept byte for byte.
pub fn merge_info(existing: Option<&[u8]>, info: &Info) -> Result<Merged, String> {
    let fresh_one = || {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"Starbase\">\n <rdf:RDF xmlns:rdf=\"{RDF}\">{}\n </rdf:RDF>\n</x:xmpmeta>\n",
            info_description(info)
        )
    };
    let Some(existing) = existing else {
        return Ok(if info.is_empty() { Merged::Same } else { Merged::Write(checked_info(fresh_one().into_bytes(), info)?) });
    };
    let (bom, text) = text_of(existing)?;
    let doc = parse(text)?;
    let rdf = rdf_root(&doc).ok_or("it has no rdf:RDF")?;
    let mut cuts: Vec<(usize, usize, String)> = Vec::new();
    for d in descriptions(rdf) {
        let ps = props(d);
        let owned: Vec<&Prop> = ps.iter().filter(|p| p.ns == Some(INFO)).collect();
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
    if info.is_empty() && cuts.is_empty() {
        return Ok(Merged::Same);
    }
    if !info.is_empty() {
        let r = rdf.range();
        if text[r.clone()].ends_with("/>") {
            let qname: String = text[r.start + 1..].chars().take_while(|c| !c.is_whitespace() && *c != '/' && *c != '>').collect();
            cuts.push((r.end - 2, r.end, format!(">{}\n </{qname}>", info_description(info))));
        } else {
            let close = text[..r.end].rfind("</").ok_or("its rdf:RDF has no end")?;
            let at = back_over_space(text, close).max(r.start);
            cuts.push((at, at, info_description(info)));
        }
    }
    cuts.sort_by_key(|c| (c.0, c.1));
    let mut out = String::with_capacity(text.len() + 1024);
    out.push_str(bom);
    let mut at = 0;
    for (start, end, with) in &cuts {
        if *start < at {
            return Err("overlapping changes".into());
        }
        out.push_str(&text[at..*start]);
        out.push_str(with);
        at = *end;
    }
    out.push_str(&text[at..]);
    if info.is_empty() && empty(&out)? {
        return Ok(Merged::Delete);
    }
    Ok(Merged::Write(checked_info(out.into_bytes(), info)?))
}

/// The merged sidecar must parse and read back as exactly the Info written.
fn checked_info(bytes: Vec<u8>, info: &Info) -> Result<Vec<u8>, String> {
    if bytes.len() as u64 > MAX_SIDECAR {
        return Err("the sidecar would be too big".into());
    }
    let (_, text) = text_of(&bytes)?;
    parse(text)?;
    if read_info(&bytes) != *info {
        return Err("the merged sidecar didn't read back as written".into());
    }
    Ok(bytes)
}
