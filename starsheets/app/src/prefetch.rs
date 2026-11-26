//! Quiet prefetch of the Docs and Slides program files from Drive.
//!
//! Once Drive has settled (a few seconds idle after it finished loading) the hashed `.js` and
//! `.wasm` files named in `/docs/` and `/slides/` `index.html` are fetched at low priority
//! (`<link rel="prefetch">`), so opening a document later is served from the browser cache.
//! Skipped on Save-Data, a slow (2g/3g) or metered/cellular connection. Done once per app
//! version: the list of files is remembered in local storage, and the hashed names change
//! whenever the program does.

use std::cell::RefCell;

/// How long Drive must have been loaded and idle before anything is fetched.
pub const SETTLE_SECS: f64 = 4.0;

/// What the browser says about the connection (all optional: Firefox gives none of it).
// Only the web build reads the connection; native builds just compile it for the tests.
#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
#[derive(Default, Clone, Debug)]
pub struct Conn {
    pub save_data: bool,
    pub effective_type: Option<String>,
    /// "cellular" and friends, when the browser tells.
    pub kind: Option<String>,
}

/// Whether a quiet background download is welcome on this connection.
#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
pub fn allowed(c: &Conn) -> bool {
    if c.save_data {
        return false;
    }
    if let Some(t) = &c.effective_type
        && matches!(t.as_str(), "slow-2g" | "2g" | "3g")
    {
        return false;
    }
    !matches!(c.kind.as_deref(), Some("cellular"))
}

/// The program files an app's `index.html` names: every `.js` and `.wasm` address in a `href` or `src`,
/// made absolute under `base` (e.g. `/docs/`), once each, in order.
#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
pub fn assets(html: &str, base: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for attr in ["href=\"", "src=\""] {
        let mut rest = html;
        while let Some(i) = rest.find(attr) {
            rest = &rest[i + attr.len()..];
            let Some(end) = rest.find('"') else { break };
            let v = &rest[..end];
            let path = v.split(['?', '#']).next().unwrap_or(v);
            if (path.ends_with(".wasm") || path.ends_with(".js")) && !path.contains("://") && !path.starts_with("//") {
                let abs = if path.starts_with('/') { path.to_string() } else { format!("{base}{}", path.trim_start_matches("./")) };
                if !out.contains(&abs) {
                    out.push(abs);
                }
            }
            rest = &rest[end..];
        }
    }
    out
}

/// Where one app's prefetch stands. (Started and Done are only set by the web build.)
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    Skipped,
    Started,
    Done,
}

impl Stage {
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    fn name(self) -> &'static str {
        match self {
            Stage::Skipped => "skipped",
            Stage::Started => "started",
            Stage::Done => "done",
        }
    }
}

/// Whether to start now: Drive has finished loading and has been idle long enough.
pub fn ready(loaded_for: f64) -> bool {
    loaded_for >= SETTLE_SECS
}

#[derive(Default)]
struct State {
    /// Seconds (page clock) when Drive was first seen loaded; None while still loading.
    loaded_at: Option<f64>,
    began: bool,
    docs: Option<Stage>,
    slides: Option<Stage>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// `{docs, slides}` for the test hook: "skipped" | "started" | "done", null before it begins.
pub fn test_state() -> serde_json::Value {
    STATE.with(|s| {
        let s = s.borrow();
        let f = |x: Option<Stage>| x.map(|x| x.name());
        serde_json::json!({ "docs": f(s.docs), "slides": f(s.slides) })
    })
}

/// Called every frame by Drive; cheap. `now` is seconds on any steady clock.
pub fn tick(ctx: &egui::Context, loading: bool, now: f64) {
    let (go, waiting) = STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.began {
            return (false, false);
        }
        if loading {
            s.loaded_at = None;
            return (false, true);
        }
        let t = *s.loaded_at.get_or_insert(now);
        if ready(now - t) {
            s.began = true;
            (true, false)
        } else {
            (false, true)
        }
    });
    if go {
        start(ctx);
    } else if waiting {
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn start(_ctx: &egui::Context) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.docs = Some(Stage::Skipped);
        s.slides = Some(Stage::Skipped);
    });
}

#[cfg(target_arch = "wasm32")]
fn conn() -> Conn {
    use js_sys::Reflect;
    use wasm_bindgen::JsValue;
    let mut c = Conn::default();
    let Some(w) = web_sys::window() else { return c };
    let nav: JsValue = w.navigator().into();
    if let Ok(n) = Reflect::get(&nav, &"connection".into())
        && n.is_object()
    {
        c.save_data = Reflect::get(&n, &"saveData".into()).ok().and_then(|v| v.as_bool()).unwrap_or(false);
        c.effective_type = Reflect::get(&n, &"effectiveType".into()).ok().and_then(|v| v.as_string());
        c.kind = Reflect::get(&n, &"type".into()).ok().and_then(|v| v.as_string());
    }
    c
}

#[cfg(target_arch = "wasm32")]
fn set(app: &str, st: Stage, ctx: &egui::Context) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if app == "docs" { s.docs = Some(st) } else { s.slides = Some(st) }
    });
    ctx.request_repaint();
}

#[cfg(target_arch = "wasm32")]
fn start(ctx: &egui::Context) {
    if !allowed(&conn()) {
        set("docs", Stage::Skipped, ctx);
        set("slides", Stage::Skipped, ctx);
        return;
    }
    for app in ["docs", "slides"] {
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move { one(app, &ctx).await });
    }
}

/// Prefetches one app (once per version) and tracks it to "done".
#[cfg(target_arch = "wasm32")]
async fn one(app: &'static str, ctx: &egui::Context) {
    use wasm_bindgen::{JsCast, closure::Closure};
    let base = format!("/{app}/");
    let Ok((200, _, bytes)) = crate::files::get_bytes(&base).await else {
        return set(app, Stage::Skipped, ctx);
    };
    let list = assets(&String::from_utf8_lossy(&bytes), &base);
    if list.is_empty() {
        return set(app, Stage::Skipped, ctx);
    }
    let key = format!("sb_prefetched_{app}");
    let sig = list.join(",");
    let store = web_sys::window().and_then(|w| w.local_storage().ok().flatten());
    if store.as_ref().and_then(|s| s.get_item(&key).ok().flatten()).as_deref() == Some(sig.as_str()) {
        return set(app, Stage::Done, ctx);
    }
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
    let left = std::rc::Rc::new(std::cell::Cell::new(list.len()));
    set(app, Stage::Started, ctx);
    for href in &list {
        let Ok(el) = doc.create_element("link") else { continue };
        let _ = el.set_attribute("rel", "prefetch");
        let _ = el.set_attribute("href", href);
        let _ = el.set_attribute("crossorigin", "anonymous");
        let _ = el.set_attribute("as", if href.ends_with(".wasm") { "fetch" } else { "script" });
        let (left, ctx, key, sig, store) = (left.clone(), ctx.clone(), key.clone(), sig.clone(), store.clone());
        let fin = Closure::<dyn FnMut()>::new(move || {
            left.set(left.get().saturating_sub(1));
            if left.get() == 0 {
                if let Some(s) = &store {
                    let _ = s.set_item(&key, &sig);
                }
                set(app, Stage::Done, &ctx);
            }
        });
        let f: &js_sys::Function = fin.as_ref().unchecked_ref();
        let _ = el.add_event_listener_with_callback("load", f);
        let _ = el.add_event_listener_with_callback("error", f);
        fin.forget();
        if let Some(head) = doc.head() {
            let _ = head.append_child(&el);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(save: bool, eff: Option<&str>, kind: Option<&str>) -> Conn {
        Conn { save_data: save, effective_type: eff.map(String::from), kind: kind.map(String::from) }
    }

    #[test]
    fn decides_by_connection() {
        assert!(allowed(&Conn::default()));
        assert!(allowed(&c(false, Some("4g"), Some("wifi"))));
        assert!(!allowed(&c(true, Some("4g"), None)));
        for t in ["slow-2g", "2g", "3g"] {
            assert!(!allowed(&c(false, Some(t), None)), "{t}");
        }
        assert!(!allowed(&c(false, Some("4g"), Some("cellular"))));
    }

    #[test]
    fn waits_for_drive_to_settle() {
        assert!(!ready(0.0));
        assert!(!ready(3.9));
        assert!(ready(4.0));
    }

    #[test]
    fn finds_the_program_files() {
        let html = r#"<head><link rel="preload" href="./stardocs-ab12_bg.wasm" as="fetch" crossorigin>
<link rel="modulepreload" href="/docs/stardocs-ab12.js"><link rel="icon" href="favicon.svg">
<link rel="stylesheet" href="https://cdn.example/x.js"><script type="module">import i from './stardocs-ab12.js?v=1';</script>
<link rel="modulepreload" href="stardocs-ab12.js"></head>"#;
        assert_eq!(assets(html, "/docs/"), vec!["/docs/stardocs-ab12_bg.wasm", "/docs/stardocs-ab12.js"]);
        assert!(assets("<p>none</p>", "/docs/").is_empty());
    }

    #[test]
    fn starts_once_after_settling() {
        let ctx = egui::Context::default();
        STATE.with(|s| *s.borrow_mut() = State::default());
        tick(&ctx, true, 0.0);
        tick(&ctx, false, 1.0);
        assert!(test_state()["docs"].is_null());
        tick(&ctx, false, 5.0);
        assert_eq!(test_state()["docs"], "skipped"); // native: nothing to fetch
        tick(&ctx, true, 6.0);
        assert!(STATE.with(|s| s.borrow().began));
    }
}
