//! Sidecars: merging keeps what isn't ours byte for byte, hostile XML is refused, files are
//! never reached through links.

use super::*;

fn edits(j: &str) -> Edits {
    serde_json::from_str(j).unwrap()
}

/// What darktable writes next to a JPEG (shortened), with a history step.
const DARKTABLE_XMP: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="XMP Core 4.4.0-Exiv2">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:darktable="http://darktable.sf.net/"
    xmp:Rating="3"
    darktable:xmp_version="5"
    darktable:history_end="1">
   <darktable:history>
    <rdf:Seq>
     <rdf:li darktable:num="0" darktable:operation="exposure" darktable:enabled="1"/>
    </rdf:Seq>
   </darktable:history>
   <!-- a comment & odd   spacing stays -->
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
"#;

/// Lightroom-style: our crs fields mixed with ones we don't own, as attributes and elements.
const LIGHTROOM_XMP: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:Exposure2012="+1.00" crs:Clarity2012="+20" crs:HasCrop="True" crs:CropLeft="0.1" crs:CropTop="0.2" crs:CropRight="0.9" crs:CropBottom="0.8" crs:CropAngle="3">
<crs:Contrast2012>+25</crs:Contrast2012>
<crs:ToneCurveName2012>Linear</crs:ToneCurveName2012>
</rdf:Description>
</rdf:RDF>
</x:xmpmeta>"#;

#[test]
fn a_new_sidecar_reads_back() {
    let e = edits(r#"{"exposure": 0.5, "contrast": 10, "crop": {"l":0.1,"t":0,"r":1,"b":0.9}, "filter": {"name":"bw","amount":1}}"#);
    let Merged::Write(b) = merge(None, Some(&e)).unwrap() else { panic!("expected a write") };
    let text = String::from_utf8(b.clone()).unwrap();
    assert!(text.contains(r#"crs:Exposure2012="+0.50""#) && text.contains(r#"crs:Contrast2012="+10""#), "{text}");
    assert!(text.contains(r#"crs:HasCrop="True""#) && text.contains(r#"crs:CropLeft="0.100000""#), "{text}");
    assert!(text.contains("starbase:Edits=\"{&quot;"), "JSON escaped in the attribute: {text}");
    let back = read(&b).unwrap();
    assert_eq!((back.source, back.edits, back.foreign.len()), (Source::Starbase, e, 0));
    assert_eq!(merge(None, None).unwrap(), Merged::Same);
    assert_eq!(merge(None, Some(&Edits::default())).unwrap(), Merged::Same, "no change: no sidecar");
}

#[test]
fn merging_keeps_everything_else_byte_for_byte() {
    let e = edits(r#"{"exposure": -1.25, "rot90": 1}"#);
    let parsed = read(DARKTABLE_XMP.as_bytes()).unwrap();
    assert_eq!((parsed.source, parsed.foreign.clone()), (Source::Nothing, vec!["darktable"]));
    let Merged::Write(m) = merge(Some(DARKTABLE_XMP.as_bytes()), Some(&e)).unwrap() else { panic!() };
    let text = String::from_utf8(m.clone()).unwrap();
    assert!(text.contains("<darktable:history>") && text.contains("odd   spacing") && text.contains(r#"xmp:Rating="3""#), "{text}");
    let back = read(&m).unwrap();
    assert_eq!((back.source, &back.edits, back.foreign.clone()), (Source::Starbase, &e, vec!["darktable"]));
    // Saved again with other edits: still one description of ours.
    let e2 = edits(r#"{"saturation": 30}"#);
    let Merged::Write(m2) = merge(Some(&m), Some(&e2)).unwrap() else { panic!() };
    let t2 = String::from_utf8(m2.clone()).unwrap();
    assert_eq!(t2.matches("starbase:Edits=").count(), 1, "{t2}");
    assert!(!t2.contains("Exposure2012"), "old fields gone: {t2}");
    // Reset: exactly the original again.
    let Merged::Write(r) = merge(Some(&m2), None).unwrap() else { panic!() };
    assert_eq!(String::from_utf8(r).unwrap(), DARKTABLE_XMP);
    assert_eq!(merge(Some(DARKTABLE_XMP.as_bytes()), None).unwrap(), Merged::Same, "nothing of ours: unchanged");
}

#[test]
fn lightroom_fields_are_imported_and_only_ours_replaced() {
    let p = read(LIGHTROOM_XMP.as_bytes()).unwrap();
    assert_eq!(p.source, Source::Crs);
    assert_eq!(p.foreign, vec!["lightroom"]);
    let j = serde_json::to_value(&p.edits).unwrap();
    assert_eq!((j["exposure"].as_f64(), j["contrast"].as_i64(), j["angle"].as_f64()), (Some(1.0), Some(25), Some(3.0)), "{j}");
    assert!((j["crop"]["l"].as_f64().unwrap() - 0.1).abs() < 1e-6, "{j}");
    let e = edits(r#"{"exposure": 0.3}"#);
    let Merged::Write(m) = merge(Some(LIGHTROOM_XMP.as_bytes()), Some(&e)).unwrap() else { panic!() };
    let t = String::from_utf8(m).unwrap();
    assert!(t.contains(r#"crs:Clarity2012="+20""#) && t.contains("<crs:ToneCurveName2012>Linear</crs:ToneCurveName2012>"), "kept: {t}");
    assert!(!t.contains("<crs:Contrast2012>") && !t.contains(r#"crs:CropAngle="3""#) && t.matches("Exposure2012").count() == 1, "ours replaced: {t}");
}

#[test]
fn a_sidecar_of_only_ours_is_deleted_on_reset() {
    let e = edits(r#"{"tint": -5}"#);
    let Merged::Write(m) = merge(None, Some(&e)).unwrap() else { panic!() };
    assert_eq!(merge(Some(&m), None).unwrap(), Merged::Delete);
    assert_eq!(merge(Some(&m), Some(&Edits::default())).unwrap(), Merged::Delete, "identity = reset");
}

#[test]
fn self_closing_rdf_and_a_byte_order_mark() {
    let s = "\u{FEFF}<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"/></x:xmpmeta>";
    let e = edits(r#"{"vibrance": 40}"#);
    let Merged::Write(m) = merge(Some(s.as_bytes()), Some(&e)).unwrap() else { panic!() };
    assert!(m.starts_with("\u{FEFF}".as_bytes()));
    assert_eq!(read(&m).unwrap().edits, e);
}

#[test]
fn hostile_or_broken_sidecars_are_refused() {
    let laughs = r#"<?xml version="1.0"?><!DOCTYPE lolz [<!ENTITY lol "lol"><!ENTITY lol2 "&lol;&lol;&lol;&lol;">]>
<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" a="&lol2;"/></rdf:RDF></x:xmpmeta>"#;
    assert!(read(laughs.as_bytes()).is_err());
    assert!(merge(Some(laughs.as_bytes()), Some(&edits(r#"{"contrast":1}"#))).is_err(), "never rewritten");
    let many = format!("<a>{}</a>", "<b/>".repeat(MAX_NODES as usize + 10));
    assert!(read(many.as_bytes()).is_err(), "too many nodes");
    assert!(read(b"\xff\xfe<\x00").is_err(), "not UTF-8");
    assert!(read(b"<a>not xmp</a>").is_err(), "no rdf:RDF");
    assert!(merge(Some(b"<a><b></a>"), None).is_err(), "broken XML");
    // A broken or oversized starbase:Edits is ignored, not trusted.
    let bad = format!(
        "<rdf:RDF xmlns:rdf=\"{RDF}\"><rdf:Description xmlns:starbase=\"{STARBASE}\" starbase:Edits=\"{{&quot;exposure&quot;:99}}\"/></rdf:RDF>"
    );
    assert_eq!(read(bad.as_bytes()).unwrap().source, Source::Nothing);
}

#[test]
fn thumbnail_keys_follow_the_edits() {
    assert_eq!(edits_hash(&Edits::default()), None);
    let a = edits_hash(&edits(r#"{"exposure":1}"#)).unwrap();
    let b = edits_hash(&edits(r#"{"exposure":2}"#)).unwrap();
    assert_ne!(a, b);
    assert_eq!(thumb_key("abc", None), "abc");
    assert_ne!(thumb_key("abc", Some(&a)), thumb_key("abc", Some(&b)));
    assert_eq!(thumb_key("abc", Some(&a)).len(), 64);
}

#[test]
fn files_are_never_reached_through_links() {
    let root = std::env::temp_dir().join(format!("starbase-sidecar-{}", crate::store::random_hex(6)));
    std::fs::create_dir_all(root.join("ann/Trip")).unwrap();
    std::fs::create_dir_all(root.join("bob")).unwrap();
    let s = Store { root: root.clone(), keep: 3, spaces: true };
    let dir = Folder::open(&s, "ann/Trip").unwrap();
    assert!(dir.read("a.jpg.xmp").unwrap().is_none());
    let stat = dir.write("a.jpg.xmp", b"hello").unwrap();
    let got = dir.read("a.jpg.xmp").unwrap().unwrap();
    assert_eq!((got.bytes.as_slice(), got.stat), (&b"hello"[..], stat));
    // A sidecar that is a link to someone else's file: never read, and a write replaces the
    // link itself, leaving Bob's file alone.
    std::fs::write(root.join("bob/secret.txt"), b"bob's").unwrap();
    std::os::unix::fs::symlink(root.join("bob/secret.txt"), root.join("ann/Trip/b.jpg.xmp")).unwrap();
    assert_eq!(dir.read("b.jpg.xmp").err(), Some(SideError::NotAFile));
    dir.write("b.jpg.xmp", b"mine").unwrap();
    assert_eq!(std::fs::read(root.join("bob/secret.txt")).unwrap(), b"bob's");
    assert!(!std::fs::symlink_metadata(root.join("ann/Trip/b.jpg.xmp")).unwrap().file_type().is_symlink());
    // A folder reached through a link isn't opened.
    std::os::unix::fs::symlink(root.join("bob"), root.join("ann/Linked")).unwrap();
    assert!(Folder::open(&s, "ann/Linked").is_none());
    // Too big: refused, not cut short.
    std::fs::write(root.join("ann/Trip/c.jpg.xmp"), vec![b' '; MAX_SIDECAR as usize + 1]).unwrap();
    assert_eq!(dir.read("c.jpg.xmp").err(), Some(SideError::TooBig));
    // A folder where the sidecar should be.
    std::fs::create_dir(root.join("ann/Trip/d.jpg.xmp")).unwrap();
    assert_eq!(dir.read("d.jpg.xmp").err(), Some(SideError::NotAFile));
    dir.remove("a.jpg.xmp").unwrap();
    dir.remove("a.jpg.xmp").unwrap();
    assert!(dir.read("a.jpg.xmp").unwrap().is_none());
    let left: Vec<String> =
        std::fs::read_dir(root.join("ann/Trip")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert!(!left.iter().any(|n| n.ends_with(".tmp")), "no temp files left: {left:?}");
    assert!(dir.read("../x").is_err() && dir.write("../x", b"").is_err(), "names only");
    let _ = std::fs::remove_dir_all(&root);
}
