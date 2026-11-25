//! Sidecars: merging keeps what isn't ours byte for byte, hostile XML is refused, files are
//! never reached through links.

use super::*;

/// The picture the sidecars in these tests belong to.
const G: Geo = Geo { width: 600, height: 400, orientation: 1 };

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
    let Merged::Write(b) = merge(None, Some(&e), &G).unwrap() else { panic!("expected a write") };
    let text = String::from_utf8(b.clone()).unwrap();
    assert!(text.contains(r#"crs:Exposure2012="+0.50""#) && text.contains(r#"crs:Contrast2012="+10""#), "{text}");
    assert!(text.contains(r#"crs:HasCrop="True""#) && text.contains(r#"crs:CropLeft="0.100000""#), "{text}");
    assert!(text.contains("starbase:Edits=\"{&quot;"), "JSON escaped in the attribute: {text}");
    assert!(
        text.contains(r#"tiff:ImageWidth="600""#) && text.contains(r#"tiff:ImageLength="400""#) && text.contains(r#"tiff:Orientation="1""#),
        "{text}"
    );
    assert!(text.contains(r#"crs:ConvertToGrayscale="True""#), "{text}");
    let rational = text.split(" crs:").skip(1).any(|f| f.split('"').nth(1).is_some_and(|v| v.contains('/')));
    assert!(!rational, "numbers as decimals, never rationals: {text}");
    let back = read(&b).unwrap();
    assert_eq!((back.source, back.edits, back.foreign.len()), (Source::Starbase, e, 0));
    assert_eq!(merge(None, None, &G).unwrap(), Merged::Same);
    assert_eq!(merge(None, Some(&Edits::default()), &G).unwrap(), Merged::Same, "no change: no sidecar");
}

#[test]
fn merging_keeps_everything_else_byte_for_byte() {
    let e = edits(r#"{"exposure": -1.25, "rot90": 1}"#);
    let parsed = read(DARKTABLE_XMP.as_bytes()).unwrap();
    assert_eq!((parsed.source, parsed.foreign.clone()), (Source::Nothing, vec!["darktable"]));
    let Merged::Write(m) = merge(Some(DARKTABLE_XMP.as_bytes()), Some(&e), &G).unwrap() else { panic!() };
    let text = String::from_utf8(m.clone()).unwrap();
    assert!(text.contains("<darktable:history>") && text.contains("odd   spacing") && text.contains(r#"xmp:Rating="3""#), "{text}");
    let back = read(&m).unwrap();
    assert_eq!((back.source, &back.edits, back.foreign.clone()), (Source::Starbase, &e, vec!["darktable"]));
    // Saved again with other edits: still one description of ours.
    let e2 = edits(r#"{"saturation": 30}"#);
    let Merged::Write(m2) = merge(Some(&m), Some(&e2), &G).unwrap() else { panic!() };
    let t2 = String::from_utf8(m2.clone()).unwrap();
    assert_eq!(t2.matches("starbase:Edits=").count(), 1, "{t2}");
    assert!(!t2.contains("Exposure2012"), "old fields gone: {t2}");
    // Reset: exactly the original again.
    let Merged::Write(r) = merge(Some(&m2), None, &G).unwrap() else { panic!() };
    assert_eq!(String::from_utf8(r).unwrap(), DARKTABLE_XMP);
    assert_eq!(merge(Some(DARKTABLE_XMP.as_bytes()), None, &G).unwrap(), Merged::Same, "nothing of ours: unchanged");
}

#[test]
fn lightroom_fields_are_imported_and_only_ours_replaced() {
    let p = read_for(LIGHTROOM_XMP.as_bytes(), Some(&G)).unwrap();
    assert_eq!(p.source, Source::Crs);
    assert_eq!(p.foreign, vec!["crs"]);
    let j = serde_json::to_value(&p.edits).unwrap();
    assert_eq!((j["exposure"].as_f64(), j["contrast"].as_i64(), j["angle"].as_f64()), (Some(1.0), Some(25), Some(3.0)), "{j}");
    assert!(!p.edits.crop.is_full(), "{j}");
    // Without the picture, Adobe's crop can't be placed: not imported.
    assert!(read(LIGHTROOM_XMP.as_bytes()).unwrap().edits.crop.is_full());
    let e = edits(r#"{"exposure": 0.3}"#);
    let Merged::Write(m) = merge(Some(LIGHTROOM_XMP.as_bytes()), Some(&e), &G).unwrap() else { panic!() };
    let t = String::from_utf8(m).unwrap();
    assert!(t.contains(r#"crs:Clarity2012="+20""#) && t.contains("<crs:ToneCurveName2012>Linear</crs:ToneCurveName2012>"), "kept: {t}");
    assert!(!t.contains("<crs:Contrast2012>") && !t.contains(r#"crs:CropAngle="3""#) && t.matches("Exposure2012").count() == 1, "ours replaced: {t}");
}

#[test]
fn a_sidecar_of_only_ours_is_deleted_on_reset() {
    let e = edits(r#"{"tint": -5}"#);
    let Merged::Write(m) = merge(None, Some(&e), &G).unwrap() else { panic!() };
    assert_eq!(merge(Some(&m), None, &G).unwrap(), Merged::Delete);
    assert_eq!(merge(Some(&m), Some(&Edits::default()), &G).unwrap(), Merged::Delete, "identity = reset");
}

#[test]
fn self_closing_rdf_and_a_byte_order_mark() {
    let s = "\u{FEFF}<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"/></x:xmpmeta>";
    let e = edits(r#"{"vibrance": 40}"#);
    let Merged::Write(m) = merge(Some(s.as_bytes()), Some(&e), &G).unwrap() else { panic!() };
    assert!(m.starts_with("\u{FEFF}".as_bytes()));
    assert_eq!(read(&m).unwrap().edits, e);
}

#[test]
fn hostile_or_broken_sidecars_are_refused() {
    let laughs = r#"<?xml version="1.0"?><!DOCTYPE lolz [<!ENTITY lol "lol"><!ENTITY lol2 "&lol;&lol;&lol;&lol;">]>
<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" a="&lol2;"/></rdf:RDF></x:xmpmeta>"#;
    assert!(read(laughs.as_bytes()).is_err());
    assert!(merge(Some(laughs.as_bytes()), Some(&edits(r#"{"contrast":1}"#)), &G).is_err(), "never rewritten");
    let many = format!("<a>{}</a>", "<b/>".repeat(MAX_NODES as usize + 10));
    assert!(read(many.as_bytes()).is_err(), "too many nodes");
    assert!(read(b"\xff\xfe<\x00").is_err(), "not UTF-8");
    assert!(read(b"<a>not xmp</a>").is_err(), "no rdf:RDF");
    assert!(merge(Some(b"<a><b></a>"), None, &G).is_err(), "broken XML");
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
    let stat = dir.write("a.jpg.xmp", b"hello", None).unwrap();
    let got = dir.read("a.jpg.xmp").unwrap().unwrap();
    assert_eq!((got.bytes.as_slice(), got.stat.clone()), (&b"hello"[..], stat.clone()));
    // Changed by another app since it was read: nothing is written or removed.
    assert!(matches!(dir.write("a.jpg.xmp", b"x", None), Err(WriteError::Changed)), "it exists now");
    assert!(matches!(dir.write("a.jpg.xmp", b"x", Some("1-2")), Err(WriteError::Changed)));
    assert!(matches!(dir.remove("a.jpg.xmp", "1-2"), Err(WriteError::Changed)));
    assert_eq!(std::fs::read(root.join("ann/Trip/a.jpg.xmp")).unwrap(), b"hello");
    // Its permissions are kept.
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(root.join("ann/Trip/a.jpg.xmp"), std::fs::Permissions::from_mode(0o600)).unwrap();
    let stat = dir.write("a.jpg.xmp", b"hello again", Some(&stat)).unwrap();
    assert_eq!(std::fs::metadata(root.join("ann/Trip/a.jpg.xmp")).unwrap().permissions().mode() & 0o777, 0o600);
    // A sidecar that is a link to someone else's file: never read, and a write replaces the
    // link itself, leaving Bob's file alone.
    std::fs::write(root.join("bob/secret.txt"), b"bob's").unwrap();
    std::os::unix::fs::symlink(root.join("bob/secret.txt"), root.join("ann/Trip/b.jpg.xmp")).unwrap();
    assert_eq!(dir.read("b.jpg.xmp").err(), Some(SideError::NotAFile));
    assert!(matches!(dir.write("b.jpg.xmp", b"mine", None), Err(WriteError::Changed)), "a link is something there");
    assert_eq!(std::fs::read(root.join("bob/secret.txt")).unwrap(), b"bob's");
    // A folder reached through a link isn't opened.
    std::os::unix::fs::symlink(root.join("bob"), root.join("ann/Linked")).unwrap();
    assert!(Folder::open(&s, "ann/Linked").is_none());
    // Too big: refused, not cut short.
    std::fs::write(root.join("ann/Trip/c.jpg.xmp"), vec![b' '; MAX_SIDECAR as usize + 1]).unwrap();
    assert_eq!(dir.read("c.jpg.xmp").err(), Some(SideError::TooBig));
    // A folder where the sidecar should be.
    std::fs::create_dir(root.join("ann/Trip/d.jpg.xmp")).unwrap();
    assert_eq!(dir.read("d.jpg.xmp").err(), Some(SideError::NotAFile));
    dir.remove("a.jpg.xmp", &stat).unwrap();
    dir.remove("a.jpg.xmp", &stat).unwrap();
    assert!(dir.read("a.jpg.xmp").unwrap().is_none());
    let left: Vec<String> =
        std::fs::read_dir(root.join("ann/Trip")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert!(!left.iter().any(|n| n.ends_with(".tmp")), "no temp files left: {left:?}");
    assert!(dir.read("../x").is_err() && dir.write("../x", b"", None).is_err(), "names only");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn many_attributes_are_refused_quickly() {
    // One element with as many attributes as fit in a sidecar: roxmltree alone took seconds.
    let attrs: String = (0..22_000).map(|i| format!(" a{i}=\"\"")).collect();
    let bomb = format!("<rdf:RDF xmlns:rdf=\"{RDF}\"><rdf:Description{attrs}/></rdf:RDF>");
    assert!(bomb.len() as u64 <= MAX_SIDECAR);
    let t = std::time::Instant::now();
    assert!(read(bomb.as_bytes()).unwrap_err().contains("too many attributes"));
    // The most allowed: full elements up to the total.
    let one: String = (0..MAX_ATTRS_PER_ELEMENT).map(|i| format!(" a{i}=\"\"")).collect();
    let full = format!("<rdf:RDF xmlns:rdf=\"{RDF}\">{}</rdf:RDF>", format!("<rdf:Description{one}/>").repeat(MAX_ATTRS / MAX_ATTRS_PER_ELEMENT));
    assert!(read(full.as_bytes()).is_ok());
    assert!(t.elapsed() < std::time::Duration::from_millis(1500), "took {:?}", t.elapsed());
    // Quoted "=" and ">" and comments don't count or confuse it.
    let ok = format!("<rdf:RDF xmlns:rdf=\"{RDF}\"><!-- {} --><rdf:Description a=\"{}>\"/></rdf:RDF>", "=".repeat(30_000), "=".repeat(30_000));
    assert!(read(ok.as_bytes()).is_ok());
}

#[test]
fn process_version_is_never_written_twice() {
    let lr = format!(
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"{RDF}\"><rdf:Description rdf:about=\"\" xmlns:crs=\"{CRS}\" crs:ProcessVersion=\"6.7\" crs:Sharpness=\"25\"/></rdf:RDF></x:xmpmeta>"
    );
    let Merged::Write(m) = merge(Some(lr.as_bytes()), Some(&edits(r#"{"contrast": 10}"#)), &G).unwrap() else { panic!() };
    let m = String::from_utf8(m).unwrap();
    assert_eq!(m.matches("ProcessVersion").count(), 1, "{m}");
    assert!(m.contains("crs:Sharpness=\"25\""), "{m}");
    // A reset takes only ours out.
    let Merged::Write(r) = merge(Some(m.as_bytes()), None, &G).unwrap() else { panic!() };
    let r = String::from_utf8(r).unwrap();
    assert!(r.contains("crs:Sharpness") && !r.contains("starbase:"), "{r}");
}

/// The interop spike's fixture sidecars (docs/photos-edit.md §4), all for a 600 × 400 picture.
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/../photo-edit/tests/fixtures/sidecars/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn the_interop_fixtures_read_as_the_spec_says() {
    let p = read_for(&fixture("crop-top-left.xmp"), Some(&G)).unwrap();
    assert_eq!((p.source, p.edits.crop), (Source::Starbase, starbase_photo_edit::Crop { l: 0.0, t: 0.0, r: 0.5, b: 0.5 }));
    let p = read_for(&fixture("angle.xmp"), Some(&G)).unwrap();
    assert_eq!((p.source, p.edits.angle), (Source::Crs, 10.0));
    let (w, h) = starbase_photo_edit::output_size(600, 400, &p.edits);
    assert!(w.abs_diff(528) <= 1 && h.abs_diff(231) <= 1, "{w} × {h}");
    // The file has EXIF orientation 6: the stored top-left quarter is the shown top-right.
    let g6 = Geo { orientation: 6, ..G };
    let p = read_for(&fixture("rot6-crop-stored-top-left.xmp"), Some(&g6)).unwrap();
    assert_eq!((p.edits.rot90, p.edits.flip_h), (0, false), "no turn of our own");
    let c = p.edits.crop;
    assert!((c.l - 0.5).abs() < 1e-3 && c.t.abs() < 1e-3 && (c.r - 1.0).abs() < 1e-3 && (c.b - 0.5).abs() < 1e-3, "{c:?}");
    assert_eq!(starbase_photo_edit::output_size(400, 600, &p.edits), (200, 300));
    let p = read_for(&fixture("tone.xmp"), Some(&G)).unwrap();
    let j = serde_json::to_value(&p.edits).unwrap();
    assert_eq!(
        j,
        serde_json::to_value(edits(
            r#"{"exposure":1,"contrast":50,"highlights":-50,"shadows":50,"saturation":-100,"vibrance":-100,"temperature":40,"tint":20,"filter":{"name":"bw","amount":1}}"#
        ))
        .unwrap()
    );
    // darktable's rewrite on mere import: ours still wins, and its automatic steps aren't edits.
    let p = read_for(&fixture("after-darktable-import.xmp"), Some(&G)).unwrap();
    assert_eq!((p.source, p.foreign.clone()), (Source::Starbase, Vec::<&str>::new()));
}

#[test]
fn what_we_write_reads_back_through_the_fixtures_rules() {
    // Our crs: crop for the fixture's edit matches the fixture (orientation 1, no angle).
    let e = edits(r#"{"crop":{"l":0,"t":0,"r":0.5,"b":0.5}}"#);
    let Merged::Write(m) = merge(None, Some(&e), &G).unwrap() else { panic!() };
    let t = String::from_utf8(m).unwrap();
    for f in [r#"crs:CropLeft="0.000000""#, r#"crs:CropRight="0.500000""#, r#"crs:CropBottom="0.500000""#, r#"tiff:Orientation="1""#] {
        assert!(t.contains(f), "{f} in {t}");
    }
    // A quarter turn of ours on an orientation-6 file: final orientation 3, and the crop of the
    // shown picture lands where the fixture rules read it back.
    let g6 = Geo { orientation: 6, ..G };
    let e = edits(r#"{"rot90":1,"angle":5,"crop":{"l":0.2,"t":0.1,"r":0.6,"b":0.7}}"#);
    let Merged::Write(m) = merge(None, Some(&e), &g6).unwrap() else { panic!() };
    let t = String::from_utf8(m.clone()).unwrap();
    assert!(t.contains(r#"tiff:Orientation="3""#), "{t}");
    // Read back from crs: alone (our JSON taken out), the same edit, near enough.
    let only_crs = t.replace(&t[t.find("   starbase:EditsVersion").unwrap()..t.find("/>").unwrap()], "");
    let p = read_for(only_crs.as_bytes(), Some(&g6)).unwrap();
    assert_eq!((p.source, p.edits.rot90, p.edits.flip_h), (Source::Crs, 1, false), "{only_crs}");
    assert!((p.edits.angle - 5.0).abs() < 1e-3 && (p.edits.crop.l - 0.2).abs() < 2e-3 && (p.edits.crop.b - 0.7).abs() < 2e-3, "{:?}", p.edits);
    // A darktable history with a real step is foreign; its defaults alone aren't.
    let dt = |ops: &str| {
        format!(
            "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"{RDF}\"><rdf:Description rdf:about=\"\" xmlns:darktable=\"http://darktable.sf.net/\"><darktable:history><rdf:Seq>{ops}</rdf:Seq></darktable:history></rdf:Description></rdf:RDF></x:xmpmeta>"
        )
    };
    let auto = r#"<rdf:li darktable:operation="colorin"/><rdf:li darktable:operation="flip" darktable:multi_name="auto"/>"#;
    assert!(read(dt(auto).as_bytes()).unwrap().foreign.is_empty());
    let user_flip = r#"<rdf:li darktable:operation="flip" darktable:multi_name=""/>"#;
    assert_eq!(read(dt(&format!("{auto}{user_flip}")).as_bytes()).unwrap().foreign, vec!["darktable"]);
    // A newer Starbase's edits are left alone.
    let newer = format!(
        "<rdf:RDF xmlns:rdf=\"{RDF}\"><rdf:Description xmlns:starbase=\"{STARBASE}\" starbase:EditsVersion=\"2\" starbase:Edits=\"{{}}\"/></rdf:RDF>"
    );
    assert!(read(newer.as_bytes()).unwrap().newer);
}

/// Runs `f` on a thread with a 256 KB stack: a parse that still recursed per nesting level
/// would overflow it (and abort the test process) on the hostile inputs below.
fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new().stack_size(256 * 1024).spawn(f).unwrap().join().unwrap()
}

#[test]
fn deep_nesting_is_refused_quickly_on_a_small_stack() {
    for n in [5_000usize, 100_000] {
        let t = std::time::Instant::now();
        let err = on_small_stack(move || {
            let bomb = format!("<rdf:RDF xmlns:rdf=\"{RDF}\">{}{}</rdf:RDF>", "<a>".repeat(n), "</a>".repeat(n));
            parse(&bomb).err()
        });
        assert!(err.unwrap().contains("nested too deeply"), "{n}");
        assert!(t.elapsed() < std::time::Duration::from_millis(500), "{n}: {:?}", t.elapsed());
    }
    // Self-closing elements, comments and closed siblings don't count as depth.
    let ok = format!("<rdf:RDF xmlns:rdf=\"{RDF}\">{}{}<!-- {} --></rdf:RDF>", "<a/>".repeat(10_000), "<b></b>".repeat(10_000), "<c>".repeat(1000));
    assert!(on_small_stack(move || parse(&ok).is_ok()));
    // The most allowed still parses on the small stack.
    let deepest = format!("<rdf:RDF xmlns:rdf=\"{RDF}\">{}{}</rdf:RDF>", "<a>".repeat(MAX_DEPTH - 1), "</a>".repeat(MAX_DEPTH - 1));
    assert!(on_small_stack(move || parse(&deepest).is_ok()));
}

#[test]
fn many_namespaces_are_refused_quickly_on_a_small_stack() {
    let t = std::time::Instant::now();
    // 9 000 nested elements each declaring one prefix: took 170 s before the cap.
    let nested: String = (0..9_000).map(|i| format!("<p{i}:a xmlns:p{i}=\"u{i}\">")).collect();
    let flat: String = (0..9_000).map(|i| format!("<a xmlns:p{i}=\"u{i}\"/>")).collect();
    let wide: String = (0..MAX_NS_PER_ELEMENT + 1).map(|i| format!(" xmlns:p{i}=\"u{i}\"")).collect();
    // Within the depth cap: 60 nested elements declaring 20 each.
    let decl = |e: usize| -> String { (0..20).map(|i| format!(" xmlns:q{e}x{i}=\"u\"")).collect() };
    let shallow: String = (0..60).map(|e| format!("<a{}>", decl(e))).collect();
    for (bomb, why) in [
        (nested, "nested too deeply"),
        (flat, "too many namespaces"),
        (format!("<a{wide}/>"), "too many namespaces"),
        (shallow, "too many namespaces"),
    ] {
        let doc = format!("<rdf:RDF xmlns:rdf=\"{RDF}\">{bomb}</rdf:RDF>");
        assert!(doc.len() as u64 <= MAX_SIDECAR);
        let err = on_small_stack(move || parse(&doc).err());
        assert!(err.as_deref().unwrap_or("").contains(why), "{err:?}");
    }
    // The most allowed, nested as deep as allowed: roxmltree copies them quickly.
    let most: String = (0..MAX_DEPTH - 1)
        .map(|e| format!("<a{}>", if e < MAX_NS / 4 - 1 { (0..4).map(|i| format!(" xmlns:r{e}x{i}=\"u\"")).collect() } else { String::new() }))
        .collect();
    let most = format!("<rdf:RDF xmlns:rdf=\"{RDF}\">{most}{}</rdf:RDF>", "</a>".repeat(MAX_DEPTH - 1));
    assert!(on_small_stack(move || parse(&most).is_ok()));
    assert!(t.elapsed() < std::time::Duration::from_millis(500), "{:?}", t.elapsed());
    // A wide but allowed set, and xmlns inside a value or text, are fine.
    let ok_wide: String = (0..MAX_NS_PER_ELEMENT - 1).map(|i| format!(" xmlns:p{i}=\"u{i}\"")).collect();
    let ok = format!("<rdf:RDF xmlns:rdf=\"{RDF}\"><a{ok_wide} v=\"{}\"> {} </a></rdf:RDF>", " xmlns".repeat(500), " xmlns".repeat(500));
    assert!(on_small_stack(move || parse(&ok).is_ok()));
}

#[test]
fn a_dtd_is_refused_by_the_prescan() {
    let doc = format!("<!DOCTYPE x [<!ENTITY a \"b\">]><rdf:RDF xmlns:rdf=\"{RDF}\"/>");
    assert!(parse(&doc).unwrap_err().contains("DTD"));
}
