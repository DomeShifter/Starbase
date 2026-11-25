//! `window.starslides`: a test hook so AI agents and scripts can drive and inspect the app without
//! clicking at pixel positions in screenshots, as StarDocs' `window.stardocs`. Only installed
//! when the page URL has a `test` parameter (`?test` or `&test`); normal visits never see it.
//! (DeckCraft's own page API, `window.deckcraft`, is always there too: `api.rs`.)
//!
//! - `starslides.run(id, params)`: a DeckCraft command id, or one of ours (`slides.*`, the menus'
//!   own: `slides.theme`, `slides.download`, `slides.new` …), as a menu would run it;
//! - `starslides.state()`: the window and presentation as JSON (below);
//! - each call returns a request number; `await starslides.wait(n)` gives its answer as JSON
//!   text, `{"ok": value}` or `{"err": "message"}`. `starslides.last` and `starslides.done` hold
//!   the newest answer and its number.
//!
//! JavaScript runs outside egui's frame, so each call only queues a request and asks for a
//! repaint; [`TestHook::process`] answers the queue at the start of the next frame.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

use deckcraft_ui_egui::SlideApp;
use serde_json::{Value, json};
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

use crate::shell::{Drive, Shell};

const KEEP_RESULTS: usize = 64;
const WAIT_MS: i32 = 10_000;
const POLL_MS: i32 = 16;

enum Request {
    Run(String, Value),
    State,
}

type Queue = Rc<RefCell<VecDeque<(u32, Request)>>>;
type Results = Rc<RefCell<BTreeMap<u32, Value>>>;

pub struct TestHook {
    queue: Queue,
    results: Results,
    object: js_sys::Object,
}

impl TestHook {
    /// Puts `window.starslides` on the page when the URL asks for it (`test` parameter).
    pub fn install(ctx: &egui::Context) -> Option<Self> {
        let window = web_sys::window()?;
        let search = window.location().search().ok()?;
        if !web_sys::UrlSearchParams::new_with_str(&search).ok()?.has("test") {
            return None;
        }
        let queue: Queue = Rc::default();
        let results: Results = Rc::default();
        let next = Rc::new(RefCell::new(0u32));
        let object = js_sys::Object::new();
        let push = {
            let queue = queue.clone();
            let ctx = ctx.clone();
            Rc::new(move |request: Request| -> u32 {
                let mut n = next.borrow_mut();
                *n += 1;
                queue.borrow_mut().push_back((*n, request));
                ctx.request_repaint();
                *n
            })
        };
        let run = {
            let push = push.clone();
            Closure::<dyn Fn(String, JsValue) -> u32>::new(move |id: String, params: JsValue| push(Request::Run(id, params_json(&params))))
        };
        let state = {
            let push = push.clone();
            Closure::<dyn Fn() -> u32>::new(move || push(Request::State))
        };
        let wait = {
            let results = results.clone();
            Closure::<dyn Fn(u32) -> js_sys::Promise>::new(move |n: u32| {
                let results = results.clone();
                wasm_bindgen_futures::future_to_promise(async move {
                    let mut waited = 0;
                    loop {
                        if let Some(v) = results.borrow().get(&n) {
                            return Ok(JsValue::from_str(&v.to_string()));
                        }
                        if waited >= WAIT_MS {
                            return Ok(JsValue::from_str(&json!({"err": format!("no answer to request {n} within {WAIT_MS} ms")}).to_string()));
                        }
                        sleep(POLL_MS).await;
                        waited += POLL_MS;
                    }
                })
            })
        };
        let set = |name: &str, value: &JsValue| {
            if let Err(e) = js_sys::Reflect::set(&object, &JsValue::from_str(name), value) {
                log::warn!("test hook: cannot set starslides.{name}: {e:?}");
            }
        };
        set("run", run.as_ref());
        set("state", state.as_ref());
        set("wait", wait.as_ref());
        set("done", &JsValue::from(0));
        set("last", &JsValue::NULL);
        run.forget();
        state.forget();
        wait.forget();
        if let Err(e) = js_sys::Reflect::set(&window, &JsValue::from_str("starslides"), &object) {
            log::warn!("test hook: cannot install window.starslides: {e:?}");
            return None;
        }
        log::info!("test hook: window.starslides is on");
        Some(Self { queue, results, object })
    }

    /// Answers every queued request. Call at the start of the frame's UI, before the shell draws.
    pub fn process(&self, app: &mut SlideApp, shell: &mut Shell, drive: Option<&dyn Drive>, ctx: &egui::Context) {
        loop {
            let Some((n, request)) = self.queue.borrow_mut().pop_front() else { break };
            let answer = match request {
                Request::Run(id, params) => shell.command(app, drive, &id, params),
                Request::State => Ok(snapshot(app, shell, drive, ctx)),
            };
            let answer = match answer {
                Ok(v) => json!({ "ok": v }),
                Err(e) => json!({ "err": e }),
            };
            let js = js_sys::JSON::parse(&answer.to_string()).unwrap_or(JsValue::NULL);
            let mut results = self.results.borrow_mut();
            results.insert(n, answer);
            while results.len() > KEEP_RESULTS {
                results.pop_first();
            }
            drop(results);
            for (name, value) in [("last", js), ("done", JsValue::from(n))] {
                if let Err(e) = js_sys::Reflect::set(&self.object, &JsValue::from_str(name), &value) {
                    log::warn!("test hook: cannot set starslides.{name}: {e:?}");
                }
            }
        }
    }
}

/// The window and presentation as JSON: what our window shows, the open presentations (title,
/// file name, unsaved changes), the slide count and current slide, the view, the drawing tool,
/// the formatting at the selection, a slideshow if one runs, and the theme.
fn snapshot(app: &mut SlideApp, shell: &Shell, drive: Option<&dyn Drive>, ctx: &egui::Context) -> Value {
    let docs: Vec<Value> = app.session.documents().iter().map(|d| json!({"title": d.title(), "dirty": d.is_dirty(), "path": d.path})).collect();
    let (slides, current) = app.session.active().map(|d| (d.doc.slides.len(), d.selection.slide)).unwrap_or((0, 0));
    let format = app.session.execute("format.state", &json!({})).unwrap_or_else(|e| json!({ "err": e.to_string() }));
    json!({
        "shell": shell.state(app, drive),
        "documents": docs,
        "slides": slides,
        "current": current,
        "view": serde_json::to_value(app.ui.view).unwrap_or_default(),
        "notes": app.ui.notes,
        "pane": app.ui.pane,
        "tool": serde_json::to_value(&app.session.tool.kind).unwrap_or_default(),
        "format": format,
        "showing": app.show.is_some(),
        "status": app.status.as_ref().map(|(m, _)| m.clone()),
        "theme": crate::theme::state(app, shell.mode, ctx),
    })
}

fn params_json(params: &JsValue) -> Value {
    if params.is_undefined() || params.is_null() {
        return json!({});
    }
    let text = match params.as_string() {
        Some(s) => s,
        None => js_sys::JSON::stringify(params).ok().and_then(|s| s.as_string()).unwrap_or_default(),
    };
    serde_json::from_str(&text).unwrap_or_else(|_| json!({}))
}

async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}
