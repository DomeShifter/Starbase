//! `window.stardocs`: a test hook so AI agents and scripts can drive and inspect the app without
//! clicking at pixel positions in screenshots. Only installed when the page URL has a `test`
//! parameter (`?test` or `&test`); normal visits never see it. How to use it: `TESTING.md`.
//!
//! JavaScript runs outside egui's frame, so each call only queues a request and asks for a
//! repaint; [`TestHook::process`] answers the queue at the start of the next frame and writes each
//! answer to `stardocs.last` (`{"ok": value}` or `{"err": "message"}`), `stardocs.done` (the
//! request number) and a short history that `stardocs.wait(n)` reads.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

use serde_json::{Value, json};
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wordcraft_ui_egui::WordApp;

use crate::server::Link;
use crate::shell::Shell;

/// Most document text a snapshot carries, in bytes.
const MAX_TEXT: usize = 100 * 1024;
/// How many answers `stardocs.wait` can still find.
const KEEP_RESULTS: usize = 64;
/// How long `stardocs.wait` waits for an answer, in milliseconds.
const WAIT_MS: i32 = 10_000;
/// How often `stardocs.wait` looks for the answer, in milliseconds (about a frame).
const POLL_MS: i32 = 16;

/// One queued call from the page.
enum Request {
    /// A WordCraft command id, `ui.dialog:<name>`, or a StarDocs `stardocs.*` menu id.
    Run(String, Value),
    /// A snapshot of the app and document.
    State,
    /// Text typed at the caret.
    Type(String),
    /// One named key (Enter, Tab, Backspace, Delete, arrows, Home, End).
    Key(String),
}

type Queue = Rc<RefCell<VecDeque<(u32, Request)>>>;
type Results = Rc<RefCell<BTreeMap<u32, Value>>>;

/// The queue the page fills and the answers it reads back.
pub struct TestHook {
    queue: Queue,
    results: Results,
    object: js_sys::Object,
    ctx: egui::Context,
}

impl TestHook {
    /// Puts `window.stardocs` on the page when the URL asks for it (`test` parameter).
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
            move |request: Request| -> u32 {
                let mut n = next.borrow_mut();
                *n += 1;
                queue.borrow_mut().push_back((*n, request));
                ctx.request_repaint();
                *n
            }
        };
        let push = Rc::new(push);
        let run = {
            let push = push.clone();
            Closure::<dyn Fn(String, JsValue) -> u32>::new(move |id: String, params: JsValue| push(Request::Run(id, params_json(&params))))
        };
        let state = {
            let push = push.clone();
            Closure::<dyn Fn() -> u32>::new(move || push(Request::State))
        };
        let type_text = {
            let push = push.clone();
            Closure::<dyn Fn(String) -> u32>::new(move |text: String| push(Request::Type(text)))
        };
        let key = {
            let push = push.clone();
            Closure::<dyn Fn(String) -> u32>::new(move |name: String| push(Request::Key(name)))
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
                log::warn!("test hook: cannot set stardocs.{name}: {e:?}");
            }
        };
        set("run", run.as_ref());
        set("state", state.as_ref());
        set("type", type_text.as_ref());
        set("key", key.as_ref());
        set("wait", wait.as_ref());
        set("done", &JsValue::from(0));
        set("last", &JsValue::NULL);
        // The functions live as long as the page.
        run.forget();
        state.forget();
        type_text.forget();
        key.forget();
        wait.forget();
        if let Err(e) = js_sys::Reflect::set(&window, &JsValue::from_str("stardocs"), &object) {
            log::warn!("test hook: cannot install window.stardocs: {e:?}");
            return None;
        }
        log::info!("test hook: window.stardocs is on");
        Some(Self { queue, results, object, ctx: ctx.clone() })
    }

    /// Answers every queued request. Call at the start of `WebShell::ui`, before the shell draws.
    pub fn process(&self, app: &mut WordApp, shell: &mut Shell, link: Option<&Link>) {
        loop {
            // Take one at a time: a command may not hold the queue while the page adds to it.
            let Some((n, request)) = self.queue.borrow_mut().pop_front() else { break };
            let answer = match request {
                Request::Run(id, params) => run(app, shell, link, &id, params),
                Request::State => Ok(snapshot(app, shell, link, &self.ctx)),
                Request::Type(text) => app.run("text.insert", json!({ "text": text })),
                Request::Key(name) => match key_command(&name) {
                    Some(id) => app.run(id, json!({})),
                    None => Err(format!("unknown key `{name}`")),
                },
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
                    log::warn!("test hook: cannot set stardocs.{name}: {e:?}");
                }
            }
        }
    }
}

/// Runs one command id as the menus would, but hands back its result or error.
fn run(app: &mut WordApp, shell: &mut Shell, link: Option<&Link>, id: &str, params: Value) -> Result<Value, String> {
    if id.starts_with("stardocs.") {
        return stardocs_command(shell, app, link, id, params);
    }
    // As `shell::run`, which returns nothing: the same `ui.dialog:` form and the same status line.
    let result = match id.strip_prefix("ui.dialog:") {
        Some(name) => app.run("ui.dialog", json!({ "name": name })),
        None => app.run(id, params),
    };
    if let Err(e) = &result {
        app.status(e.clone());
    }
    result
}

/// StarDocs' own menu ids (`stardocs.*`).
fn stardocs_command(shell: &mut Shell, app: &mut WordApp, link: Option<&Link>, id: &str, params: Value) -> Result<Value, String> {
    shell.test_command(app, link, id, params)
}

/// The app and document as JSON: what the window shows, the formatting and view at the caret,
/// the word count, the plain text (first 100 KB), unsaved changes and the status line.
fn snapshot(app: &mut WordApp, shell: &mut Shell, link: Option<&Link>, ctx: &egui::Context) -> Value {
    let shell_state = shell.test_state(app, link);
    let mut engine = |id: &str| app.session.run(id, &json!({})).unwrap_or_else(|e| json!({ "err": e.to_string() }));
    let format = engine("format.state");
    let view = engine("view.state");
    let words = engine("review.wordCount");
    let mut text = engine("document.text").get("text").and_then(Value::as_str).unwrap_or_default().to_string();
    let truncated = text.len() > MAX_TEXT;
    if truncated {
        let mut end = MAX_TEXT;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    json!({
        "shell": shell_state,
        "format": format,
        "view": view,
        "words": words,
        "text": text,
        "textTruncated": truncated,
        "dirty": app.session.dirty,
        "status": app.session.status,
        "theme": crate::theme::state(ctx),
    })
}

/// Command parameters from the page: a JSON string, an object, or nothing (`{}`).
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

/// The engine command a named key runs.
fn key_command(name: &str) -> Option<&'static str> {
    Some(match name {
        "Enter" => "text.newParagraph",
        "Tab" => "text.tab",
        "Backspace" => "text.backspace",
        "Delete" => "text.delete",
        "Left" | "ArrowLeft" => "caret.left",
        "Right" | "ArrowRight" => "caret.right",
        "Up" | "ArrowUp" => "caret.up",
        "Down" | "ArrowDown" => "caret.down",
        "Home" => "caret.home",
        "End" => "caret.end",
        "ctrl+Home" => "caret.docStart",
        "ctrl+End" => "caret.docEnd",
        _ => return None,
    })
}

async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}
