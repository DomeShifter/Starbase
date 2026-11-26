//! Pictures for Google's IMAGE function: the grid (GridCraft `formula_graphics`, StarSheets 0102)
//! asks for each web address it meets; the browser fetches it here and hands the bytes back.
//! Only pictures the other site lets a web page read (CORS) arrive; anything else shows a small
//! broken-picture mark. The desktop app doesn't fetch them yet.

use gridcraft_ui_egui::formula_graphics;

/// Largest picture taken, in bytes.
#[cfg(target_arch = "wasm32")]
const MAX_BYTES: usize = 16 << 20;

/// Starts a fetch for every address the grid asked for since the last frame.
/// In a spreadsheet that isn't the person's own, nothing is fetched until they allow it
/// (external.rs): the cell shows a small "blocked" picture and the address waits.
pub fn fetch_wanted(ctx: &egui::Context) {
    let wanted = formula_graphics::wanted();
    if wanted.is_empty() {
        return;
    }
    if crate::external::allowed() {
        fetch_now(wanted, ctx);
        return;
    }
    for url in wanted {
        let blocked = Some((crate::external::BLOCKED_PNG.to_vec(), "image/png".to_string()));
        formula_graphics::set(&url, if crate::external::hold(&url) { blocked } else { None });
    }
    ctx.request_repaint();
}

/// Fetches these addresses now (the person's own spreadsheet, or one they allowed).
pub fn fetch_now(urls: Vec<String>, ctx: &egui::Context) {
    for url in urls {
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            formula_graphics::set(&url, None);
            continue;
        }
        #[cfg(target_arch = "wasm32")]
        {
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let got = fetch(&url).await;
                formula_graphics::set(&url, got);
                ctx.request_repaint();
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            formula_graphics::set(&url, None);
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn fetch(url: &str) -> Option<(Vec<u8>, String)> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    let window = web_sys::window()?;
    let resp: web_sys::Response = JsFuture::from(window.fetch_with_str(url)).await.ok()?.dyn_into().ok()?;
    if !resp.ok() {
        return None;
    }
    let mime = resp.headers().get("content-type").ok().flatten().unwrap_or_default();
    if !mime.starts_with("image/") {
        return None;
    }
    let buf = JsFuture::from(resp.array_buffer().ok()?).await.ok()?;
    let bytes = js_sys::Uint8Array::new(&buf).to_vec();
    (bytes.len() <= MAX_BYTES).then_some((bytes, mime.split(';').next().unwrap_or("image/png").trim().to_string()))
}
