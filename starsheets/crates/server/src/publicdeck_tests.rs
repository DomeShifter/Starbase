//! Public links to a .pptx: the signed-out slide viewer (publicdeck.rs), with and without a
//! password (the deck's pictures must load with the unlock cookie), and never the file itself.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "tests")]

use std::io::Write;

use serde_json::json;

use crate::publiclink::{grant_in, serve, unlock, update};
use crate::store::{Store, random_hex};

const NS: &str = r#"xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

fn deck() -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default();
    let mut put = |name: &str, body: &[u8]| {
        z.start_file(name, o).unwrap();
        z.write_all(body).unwrap();
    };
    put(
        "ppt/presentation.xml",
        format!(
            r#"<p:presentation {NS}><p:sldIdLst><p:sldId id="256" r:id="rId1"/><p:sldId id="257" r:id="rId2"/></p:sldIdLst><p:sldSz cx="9144000" cy="5143500"/></p:presentation>"#
        )
        .as_bytes(),
    );
    put(
        "ppt/_rels/presentation.xml.rels",
        format!(
            r#"<Relationships xmlns="{RELS}"><Relationship Id="rId1" Type="x/slide" Target="slides/slide1.xml"/><Relationship Id="rId2" Type="x/slide" Target="slides/slide2.xml"/></Relationships>"#
        )
        .as_bytes(),
    );
    put(
        "ppt/slides/slide1.xml",
        format!(
            r#"<p:sld {NS}><p:cSld><p:spTree><p:sp><p:spPr><a:xfrm><a:off x="457200" y="457200"/><a:ext cx="4572000" cy="914400"/></a:xfrm></p:spPr><p:txBody><a:p><a:r><a:rPr sz="4000"/><a:t>Hello &lt;script&gt;alert(1)&lt;/script&gt;</a:t></a:r></a:p></p:txBody></p:sp><p:pic><p:blipFill><a:blip r:embed="rId9"/></p:blipFill><p:spPr><a:xfrm><a:off x="0" y="1371600"/><a:ext cx="2000000" cy="2000000"/></a:xfrm></p:spPr></p:pic></p:spTree></p:cSld></p:sld>"#
        )
        .as_bytes(),
    );
    put(
        "ppt/slides/_rels/slide1.xml.rels",
        format!(r#"<Relationships xmlns="{RELS}"><Relationship Id="rId9" Type="x/image" Target="../media/image1.png"/></Relationships>"#).as_bytes(),
    );
    put(
        "ppt/slides/slide2.xml",
        format!(
            r#"<p:sld {NS}><p:cSld><p:spTree><p:sp><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="4572000" cy="914400"/></a:xfrm></p:spPr><p:txBody><a:p><a:r><a:t>Second slide</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#
        )
        .as_bytes(),
    );
    put("ppt/media/image1.png", b"\x89PNG-fake-bytes");
    z.finish().unwrap().into_inner()
}

/// A store with Talk.pptx and a public link to it set by `public`: (store, token).
fn linked(public: serde_json::Value) -> (Store, String) {
    let dir = std::env::temp_dir().join(format!("starsheets-publicdeck-{}", random_hex(6)));
    std::fs::create_dir_all(&dir).unwrap();
    let st = Store { root: dir.clone(), keep: 3, spaces: false };
    std::fs::create_dir_all(st.meta()).unwrap();
    std::fs::write(dir.join("Talk.pptx"), deck()).unwrap();
    let id = st.synced_index().unwrap().into_iter().find(|(_, p)| p == "Talk.pptx").unwrap().0;
    let mut p = json!({ "on": true });
    p.as_object_mut().unwrap().extend(public.as_object().unwrap().clone());
    update(&st, &id, &json!({ "public": p })).unwrap();
    let token = crate::publiclink::describe(&st, &id)["token"].as_str().unwrap().to_string();
    (st, token)
}

#[test]
fn open_view_only_link_shows_the_viewer_and_never_the_file() {
    let (st, t) = linked(json!({ "no_download": true }));
    let r = serve(&st, &t, "", "");
    let html = String::from_utf8(r.body).unwrap();
    assert_eq!(r.code, 200);
    assert!(r.script, "the viewer's own script needs the script CSP");
    assert!(
        html.contains("1 / 2")
            && html.contains("data-testid=sb-next")
            && html.contains("data-testid=sb-prev")
            && html.contains("data-testid=sb-full")
    );
    assert_eq!(html.matches("<section").count(), 2);
    assert!(html.contains("Second slide") && html.contains("/media/0"));
    assert!(!html.contains("<script>alert") && html.contains("&lt;script&gt;"), "deck text is escaped");
    assert!(!html.contains("/download") && !html.contains("Download"), "view only: no download");
    assert_eq!(serve(&st, &t, "download", "").code, 403);
    let js = serve(&st, &t, "viewer.js", "");
    assert!(js.content_type.starts_with("text/javascript") && String::from_utf8(js.body).unwrap().contains("ArrowRight"));
    let img = serve(&st, &t, "media/0", "");
    assert_eq!((img.code, img.content_type), (200, "image/png"));
    assert_eq!(serve(&st, &t, "media/7", "").code, 404);
}

#[test]
fn password_deck_shows_its_pictures_with_the_unlock_cookie() {
    let (st, t) = linked(json!({ "password": "open sesame" }));
    let locked = serve(&st, &t, "", "");
    assert!(locked.form && !locked.script, "the password page first");
    assert!(serve(&st, &t, "media/0", "").form, "no picture without the cookie");
    assert!(unlock(&st, &t, "wrong").form);
    let cookie = unlock(&st, &t, "open sesame").set_cookie.unwrap();
    assert!(cookie.contains("HttpOnly") && cookie.contains(&format!("Path=/api/public/{t}")), "{cookie}");
    let grant = grant_in(cookie.split(';').next().unwrap(), &t);
    assert!(serve(&st, &t, "", &grant).script);
    let img = serve(&st, &t, "media/0", &grant);
    assert_eq!((img.code, img.content_type), (200, "image/png"));
}
