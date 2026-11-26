//! A public link to a presentation (`.pptx`) opens a signed-out, view-only slide viewer: one
//! slide at a time, left/right keys or a click to move, "n / N", a full-screen button. No account,
//! no editing, and never the file itself (the link's "no download" switch only hides the Download
//! button; the viewer never hands out the .pptx).
//!
//! The page is built here from the file's XML (text boxes and pictures at their places; shapes,
//! charts and animations are not drawn). All text is escaped. The one script is a fixed file,
//! `/api/public/TOKEN/viewer.js`, served from this site (`script-src 'self'`): nothing from the
//! deck ever becomes script. Pictures come from `/api/public/TOKEN/media/N`, the N-th picture in
//! the file (png/jpeg/gif/webp only); those requests carry the link's unlock cookie, which is why
//! the page's sandbox keeps `allow-same-origin`.

use std::io::Read;

use roxmltree::{Document, Node};

use crate::preview::esc;

const MAX_SLIDES: usize = 400;
const MAX_XML: u64 = 8 * 1024 * 1024;
const MAX_PICTURE: u64 = 32 * 1024 * 1024;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

type Zip = zip::ZipArchive<std::io::Cursor<Vec<u8>>>;

pub fn open(data: Vec<u8>) -> Option<Zip> {
    zip::ZipArchive::new(std::io::Cursor::new(data)).ok()
}

fn read(z: &mut Zip, name: &str, max: u64) -> Option<Vec<u8>> {
    let f = z.by_name(name).ok()?;
    if f.size() > max {
        return None;
    }
    let mut out = Vec::new();
    f.take(max).read_to_end(&mut out).ok()?;
    Some(out)
}

fn xml(z: &mut Zip, name: &str) -> Option<String> {
    String::from_utf8(read(z, name, MAX_XML)?).ok()
}

/// (id, type, full path) of a part's relationships, targets resolved against the part's folder.
fn rels(z: &mut Zip, part: &str) -> Vec<(String, String, String)> {
    let (dir, file) = part.rsplit_once('/').unwrap_or(("", part));
    let Some(text) = xml(z, &format!("{dir}/_rels/{file}.rels")) else { return Vec::new() };
    let Ok(doc) = Document::parse(&text) else { return Vec::new() };
    doc.descendants()
        .filter(|n| n.has_tag_name("Relationship"))
        .filter_map(|n| {
            let t = n.attribute("Target")?;
            let full = if let Some(abs) = t.strip_prefix('/') {
                abs.to_string()
            } else {
                let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
                for seg in t.split('/') {
                    match seg {
                        ".." => {
                            parts.pop();
                        }
                        "." | "" => {}
                        s => parts.push(s),
                    }
                }
                parts.join("/")
            };
            Some((n.attribute("Id")?.to_string(), n.attribute("Type").unwrap_or("").to_string(), full))
        })
        .collect()
}

fn picture_type(name: &str) -> Option<&'static str> {
    match name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).as_deref() {
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

/// The pictures of the file, sorted by name: (zip name, type). `/media/N` is index N of this list.
fn pictures(z: &mut Zip) -> Vec<(String, &'static str)> {
    let mut v: Vec<(String, &'static str)> = (0..z.len())
        .filter_map(|i| z.by_index_raw(i).ok().map(|f| f.name().to_string()))
        .filter(|n| n.starts_with("ppt/media/"))
        .filter_map(|n| picture_type(&n).map(|t| (n, t)))
        .collect();
    v.sort();
    v
}

/// Picture N's bytes and type (only from `ppt/media/`).
pub fn media(z: &mut Zip, n: usize) -> Option<(Vec<u8>, &'static str)> {
    let (name, t) = pictures(z).into_iter().nth(n)?;
    Some((read(z, &name, MAX_PICTURE)?, t))
}

fn child<'a>(n: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    n.children().find(|c| c.has_tag_name(name))
}

/// (x, y, w, h) in EMU of a shape, from its own `a:xfrm`.
fn frame(n: Node) -> Option<(f64, f64, f64, f64)> {
    let x = n.descendants().find(|d| d.has_tag_name("xfrm"))?;
    let (o, e) = (child(x, "off")?, child(x, "ext")?);
    let g = |n: Node, a: &str| n.attribute(a).and_then(|v| v.parse::<f64>().ok());
    Some((g(o, "x")?, g(o, "y")?, g(e, "cx")?, g(e, "cy")?))
}

fn placeholder(n: Node) -> Option<(String, String)> {
    let ph = n.descendants().find(|d| d.has_tag_name("ph"))?;
    Some((ph.attribute("type").unwrap_or("").to_string(), ph.attribute("idx").unwrap_or("").to_string()))
}

/// A placeholder's frame, taken from the slide's layout when the slide doesn't give one.
fn layout_frame(layout: &Document, ph: &(String, String)) -> Option<(f64, f64, f64, f64)> {
    layout
        .descendants()
        .filter(|n| n.has_tag_name("sp"))
        .find(|n| placeholder(*n).is_some_and(|p| (!ph.1.is_empty() && p.1 == ph.1) || (ph.1.is_empty() && p.0 == ph.0)))
        .and_then(frame)
}

fn slide_html(z: &mut Zip, part: &str, base: &str, size: (f64, f64), pics: &[(String, &'static str)]) -> String {
    let Some(text) = xml(z, part) else { return String::new() };
    let Ok(doc) = Document::parse(&text) else { return String::new() };
    let rl = rels(z, part);
    let layout_text = rl.iter().find(|r| r.1.ends_with("/slideLayout")).and_then(|r| xml(z, &r.2));
    let layout = layout_text.as_deref().and_then(|t| Document::parse(t).ok());
    let pct = |v: f64, of: f64| format!("{:.3}%", v / of * 100.0);
    let mut out = String::new();
    for n in doc.descendants().filter(|n| n.has_tag_name("sp") || n.has_tag_name("pic")) {
        let fr = frame(n).or_else(|| layout.as_ref().zip(placeholder(n)).and_then(|(l, p)| layout_frame(l, &p)));
        let Some((x, y, w, h)) = fr else { continue };
        let style = format!("left:{};top:{};width:{};height:{}", pct(x, size.0), pct(y, size.1), pct(w, size.0), pct(h, size.1));
        if n.has_tag_name("pic") {
            let rid = n.descendants().find(|d| d.has_tag_name("blip")).and_then(|b| b.attribute((REL, "embed")));
            let target = rid.and_then(|r| rl.iter().find(|x| x.0 == r)).map(|r| r.2.clone());
            if let Some(i) = target.and_then(|t| pics.iter().position(|p| p.0 == t)) {
                out += &format!("<img class=o alt=\"\" style=\"{style}\" src=\"{base}/media/{i}\">");
            }
            continue;
        }
        let mut paras = String::new();
        for p in n.descendants().filter(|d| d.has_tag_name("p")) {
            let mut line = String::new();
            let mut sz: f64 = 0.0;
            for r in p.children().filter(|c| c.has_tag_name("r")) {
                if let Some(t) = child(r, "t") {
                    line += &esc(t.text().unwrap_or(""));
                }
                if let Some(s) = child(r, "rPr").and_then(|r| r.attribute("sz")).and_then(|s| s.parse::<f64>().ok()) {
                    sz = sz.max(s);
                }
            }
            if line.is_empty() {
                continue;
            }
            // Font size relative to the slide's width, so it scales with the page.
            let fs = if sz > 0.0 { sz / 100.0 * 12700.0 / size.0 * 100.0 } else { 2.2 };
            paras += &format!("<p style=\"font-size:{fs:.3}cqw\">{line}</p>");
        }
        if !paras.is_empty() {
            out += &format!("<div class=o style=\"{style}\">{paras}</div>");
        }
    }
    out
}

/// The viewer page for the deck in `data`, or None when it isn't a readable presentation.
pub fn viewer(data: Vec<u8>, title: &str, base: &str) -> Option<String> {
    let mut z = open(data)?;
    let pres = xml(&mut z, "ppt/presentation.xml")?;
    let pdoc = Document::parse(&pres).ok()?;
    let sz = pdoc.descendants().find(|d| d.has_tag_name("sldSz"))?;
    let size = (sz.attribute("cx")?.parse::<f64>().ok()?.max(1.0), sz.attribute("cy")?.parse::<f64>().ok()?.max(1.0));
    let prel = rels(&mut z, "ppt/presentation.xml");
    let ids: Vec<String> = pdoc
        .descendants()
        .filter(|d| d.has_tag_name("sldId"))
        .filter_map(|d| d.attribute((REL, "id")).map(str::to_string))
        .take(MAX_SLIDES)
        .collect();
    let pics = pictures(&mut z);
    let mut slides = String::new();
    let mut count = 0;
    for id in ids {
        let Some(part) = prel.iter().find(|r| r.0 == id).map(|r| r.2.clone()) else { continue };
        count += 1;
        slides += &format!("<section class=s hidden>{}</section>", slide_html(&mut z, &part, base, size, &pics));
    }
    if count == 0 {
        return None;
    }
    Some(format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>{t}</title>\
         <style>html,body{{margin:0;height:100%;background:#111;color:#eee;font:14px system-ui,sans-serif}}\
         #stage{{position:fixed;inset:0 0 2.6rem 0;display:flex;align-items:center;justify-content:center;cursor:pointer}}\
         .s{{position:relative;background:#fff;color:#000;container-type:inline-size;aspect-ratio:{cx}/{cy};width:min(100%,calc((100vh - 2.6rem)*{cx}/{cy}));overflow:hidden}}\
         .s[hidden]{{display:none}}.o{{position:absolute;overflow:hidden}}.o p{{margin:0 0 .3em}}img.o{{object-fit:contain}}\
         #bar{{position:fixed;left:0;right:0;bottom:0;height:2.6rem;display:flex;gap:.6rem;align-items:center;justify-content:center}}\
         #bar button{{padding:.3rem .8rem;border:0;border-radius:.3rem;background:#333;color:#eee;font:inherit;cursor:pointer}}</style>\
         <div id=stage data-testid=sb-stage>{slides}</div>\
         <div id=bar><button id=sb-prev data-testid=sb-prev aria-label=Previous>&larr;</button>\
         <span id=sb-count data-testid=sb-count>1 / {count}</span>\
         <button id=sb-next data-testid=sb-next aria-label=Next>&rarr;</button>\
         <button id=sb-full data-testid=sb-full>Full screen</button></div>\
         <script src=\"{base}/viewer.js\"></script>",
        t = esc(title),
        cx = size.0 as u64,
        cy = size.1 as u64,
    ))
}

/// The viewer's only script (fixed text, nothing from the deck).
pub const SCRIPT: &str = r#"(function(){
var s=[].slice.call(document.querySelectorAll('.s')),i=0,c=document.getElementById('sb-count');
function show(n){i=Math.max(0,Math.min(s.length-1,n));s.forEach(function(e,k){e.hidden=k!==i});c.textContent=(i+1)+' / '+s.length;}
function next(){show(i+1)}function prev(){show(i-1)}
function full(){var d=document;if(d.fullscreenElement){d.exitFullscreen()}else if(d.documentElement.requestFullscreen){d.documentElement.requestFullscreen()}}
document.getElementById('sb-next').onclick=function(e){e.stopPropagation();next()};
document.getElementById('sb-prev').onclick=function(e){e.stopPropagation();prev()};
document.getElementById('sb-full').onclick=function(e){e.stopPropagation();full()};
document.getElementById('stage').onclick=next;
document.addEventListener('keydown',function(e){
 if(e.key==='ArrowRight'||e.key==='PageDown'||e.key===' '){next();e.preventDefault()}
 else if(e.key==='ArrowLeft'||e.key==='PageUp'){prev();e.preventDefault()}
 else if(e.key==='Home'){show(0)}else if(e.key==='End'){show(s.length-1)}
 else if(e.key==='f'){full()}});
show(0);
})();"#;
