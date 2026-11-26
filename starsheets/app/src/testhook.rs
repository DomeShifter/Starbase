//! `window.starsheets`: a test hook so AI agents and scripts can drive and inspect the app
//! without clicking at pixel positions in screenshots. Only installed when the page URL has a
//! `test` parameter (`?test` or `&test`); normal visits never see it. How to use it: `TESTING.md`.
//! It mirrors StarDocs' hook (stardocs/app/src/testhook.rs).
//!
//! JavaScript runs outside egui's frame, so each call only queues a request and asks for a
//! repaint; [`TestHook::process`] answers the queue at the start of the next frame and writes each
//! answer to `starsheets.last` (`{"ok": value}` or `{"err": "message"}`), `starsheets.done` (the
//! request number) and a short history that `starsheets.wait(n)` reads.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

use gridcraft_engine::model::style::Underline;
use gridcraft_ui_egui::SheetApp;
use serde_json::{Value, json};
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

use crate::files::Files;
use crate::home::Home;

/// How many answers `starsheets.wait` can still find.
const KEEP_RESULTS: usize = 64;
/// How long `starsheets.wait` waits for an answer, in milliseconds.
const WAIT_MS: i32 = 10_000;
/// How often `starsheets.wait` looks for the answer, in milliseconds (about a frame).
const POLL_MS: i32 = 16;

/// One queued call from the page.
enum Request {
    /// A menu or toolbar id (`panel:`, `ui:` or a GridCraft command), or a `home:` id.
    Run(String, Value),
    /// A snapshot of the app.
    State,
    /// Text entered into the active cell.
    Type(String),
    /// One key or shortcut (`Enter`, `shift+Tab`, `ctrl+b` …).
    Key(String),
}

type Queue = Rc<RefCell<VecDeque<(u32, Request)>>>;
type Results = Rc<RefCell<BTreeMap<u32, Value>>>;

/// The queue the page fills and the answers it reads back.
pub struct TestHook {
    queue: Queue,
    results: Results,
    object: js_sys::Object,
}

/// What the hook works on, borrowed from the browser shell for one frame (web.rs).
pub struct Parts<'a> {
    pub app: &'a mut SheetApp,
    pub home: &'a mut Home,
    pub files: &'a Files,
    pub compact: bool,
}

impl TestHook {
    /// Puts `window.starsheets` on the page when the URL asks for it (`test` parameter).
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
                log::warn!("test hook: cannot set starsheets.{name}: {e:?}");
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
        if let Err(e) = js_sys::Reflect::set(&window, &JsValue::from_str("starsheets"), &object) {
            log::warn!("test hook: cannot install window.starsheets: {e:?}");
            return None;
        }
        log::info!("test hook: window.starsheets is on");
        Some(Self { queue, results, object })
    }

    /// Answers every queued request. Call at the start of `WebShell::ui`, before anything draws.
    pub fn process(&self, ctx: &egui::Context, p: Parts) {
        loop {
            // Take one at a time: a command may not hold the queue while the page adds to it.
            let Some((n, request)) = self.queue.borrow_mut().pop_front() else { break };
            let answer = match request {
                Request::Run(id, params) => run(ctx, &mut *p.app, &mut *p.home, &id, params),
                Request::State => Ok(snapshot(ctx, &p)),
                Request::Type(text) => type_text(p.app, p.home.active, &text),
                Request::Key(name) => key(p.app, p.home.active, &name),
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
                    log::warn!("test hook: cannot set starsheets.{name}: {e:?}");
                }
            }
        }
    }
}

/// Runs one id as the menus and toolbar would, but hands back a result or an error.
fn run(ctx: &egui::Context, app: &mut SheetApp, home: &mut Home, id: &str, params: Value) -> Result<Value, String> {
    // The avatar menu's dialogs (account.rs), on either screen.
    if id == "account:menu" {
        crate::account::open_menu(ctx)?;
        return Ok(json!({ "opened": id }));
    }
    if let Some(what) = id.strip_prefix("account:") {
        crate::account::request(ctx, what)?;
        return Ok(json!({ "opened": id }));
    }
    if id.starts_with("home:") {
        return if home.active { home.test_run(ctx, id, &params) } else { Err("not on the home screen".into()) };
    }
    if home.active {
        return Err(format!("`{id}` needs a spreadsheet open (this is the home screen)"));
    }
    // External content bar (external.rs): `external:allow` presses Allow.
    if id == "external:allow" {
        crate::external::allow(ctx);
        return Ok(json!({ "allowed": crate::external::allowed() }));
    }
    // The clipboard, which a script can't reach: `clip:copy` copies the selection as Ctrl+C does
    // and answers the text and HTML it puts on the clipboard; `clip:paste {text, html?, values?}`
    // pastes as Ctrl+V (Ctrl+Shift+V with `values`) does when the browser hands those over.
    if id == "clip:copy" {
        return app.run("edit.copy", json!({"html": true}));
    }
    if id == "clip:paste" {
        let text = params.get("text").and_then(Value::as_str).ok_or("clip:paste needs `text`")?.to_string();
        let html = params.get("html").and_then(Value::as_str).map_or(JsValue::NULL, JsValue::from_str);
        if let Some(w) = web_sys::window() {
            let _ = js_sys::Reflect::set(&w, &"__ssPasteHtml".into(), &html);
        }
        if params.get("values").and_then(Value::as_bool) == Some(true) {
            crate::shell::paste_values(app, &text);
        } else {
            app.paste_external(text);
        }
        return match app.message.take() {
            Some((_, text)) => Err(text),
            None => Ok(json!({ "done": id })),
        };
    }
    let entry = id.starts_with("panel:") || id.starts_with("ui:") || matches!(id, "edit.copy" | "edit.cut" | "edit.paste");
    // Menu entries that run engine commands themselves (Sort sheet, Freeze…) answer like commands.
    if crate::menuactions::handles(id) {
        crate::shell::run_entry_with(app, ctx, id, params);
        return match app.message.take() {
            Some((title, text)) if title != "Trim whitespace" => Err(text),
            Some((_, text)) => Ok(json!({ "done": id, "message": text })),
            None => Ok(json!({ "done": id })),
        };
    }
    if entry {
        // Panels, dialogs and the clipboard: the menus' own path (they answer nothing back).
        // The filter view bar's ✕ and the Data › Filter views list are not menu table rows but
        // run through the same path (shell/filterviews.rs).
        let bar = crate::shell::filterviews::handles(id);
        if !bar && !crate::shell::entry_ids().contains(&id) && !id.starts_with("edit.") {
            return Err(format!("`{id}` is not on any menu or toolbar"));
        }
        crate::shell::run_entry_with(app, ctx, id, params);
        return Ok(json!({ "opened": id }));
    }
    // An engine command, with parameters as the toolbar passes them; errors come back as text
    // (the menus show them as an alert instead).
    app.run(id, params)
}

/// Enters `text` into the active cell as typing it and pressing Enter would, without moving
/// (use `key('Enter')` to move down). A leading `=` makes it a formula.
fn type_text(app: &mut SheetApp, on_home: bool, text: &str) -> Result<Value, String> {
    if on_home {
        return Err("no spreadsheet open (this is the home screen)".into());
    }
    let cell = app.session.active().map(|d| d.selection.active.a1()).ok_or("no spreadsheet open")?;
    app.run("cell.set", json!({ "cell": cell, "input": text }))
}

/// One key or shortcut, as the command it runs in Sheets.
fn key(app: &mut SheetApp, on_home: bool, name: &str) -> Result<Value, String> {
    if on_home {
        return Err("no spreadsheet open (this is the home screen)".into());
    }
    let lower = name.to_lowercase().replace("arrow", "").replace("cmd+", "ctrl+");
    // Arrows: Shift extends the selection, Ctrl jumps to the edge of the data.
    let (mods, last) = lower.rsplit_once('+').unwrap_or(("", lower.as_str()));
    let arrow = match last {
        "up" => Some((-1, 0)),
        "down" => Some((1, 0)),
        "left" => Some((0, -1)),
        "right" => Some((0, 1)),
        _ => None,
    };
    if let Some((dr, dc)) = arrow
        && matches!(mods, "" | "shift" | "ctrl" | "ctrl+shift")
    {
        let params = json!({ "dr": dr, "dc": dc, "extend": mods.contains("shift"), "jump": mods.contains("ctrl") });
        return app.run("selection.move", params);
    }
    let mv = |dr: i32, dc: i32| ("selection.move", json!({ "dr": dr, "dc": dc }));
    let (id, params) = match lower.as_str() {
        "enter" => mv(1, 0),
        "shift+enter" => mv(-1, 0),
        "tab" => mv(0, 1),
        "shift+tab" => mv(0, -1),
        "ctrl+home" => ("selection.set", json!({ "range": "A1" })),
        "delete" | "backspace" => ("edit.clearContents", json!({})),
        "ctrl+z" => ("edit.undo", json!({})),
        "ctrl+y" | "ctrl+shift+z" => ("edit.redo", json!({})),
        "ctrl+a" => ("edit.selectAll", json!({})),
        "ctrl+b" => ("home.bold", json!({})),
        "ctrl+i" => ("home.italic", json!({})),
        "ctrl+u" => ("home.underline", json!({})),
        "alt+shift+5" => ("home.strikethrough", json!({})),
        _ => return Err(format!("unknown key `{name}`")),
    };
    app.run(id, params)
}

/// The app as JSON: which screen, and on a spreadsheet the file, sheets, selection, the active
/// cell's value and formula, its main formatting, saving and the other people in the file; on the
/// home screen what it lists.
/// The IMAGE addresses on the active sheet whose pictures have arrived.
/// Non-empty cells (A1, input text) inside the selection, at most 2000, row by row.
fn selection_cells(sheet: &gridcraft_engine::model::Sheet, ranges: &[gridcraft_engine::core::RangeRef]) -> Vec<Value> {
    use gridcraft_engine::core::CellRef;
    let mut out = Vec::new();
    let Some(used) = sheet.used_range() else { return out };
    for r in ranges {
        for row in r.start.row.max(used.start.row)..=r.end.row.min(used.end.row) {
            for col in r.start.col.max(used.start.col)..=r.end.col.min(used.end.col) {
                let at = CellRef { row, col };
                let t = sheet.input_text(at);
                if !t.is_empty() {
                    out.push(json!([at.a1(), t]));
                    if out.len() >= 2000 {
                        return out;
                    }
                }
            }
        }
    }
    out
}

fn formula_images_ready(app: &SheetApp) -> Vec<String> {
    let Some(sh) = app.session.active().and_then(|d| d.wb.active()) else { return Vec::new() };
    sh.cells
        .iter()
        .filter_map(|(_, c)| c.formula.as_ref().map(|f| f.text.clone()))
        .filter_map(|t| t.split('"').nth(1).filter(|_| t.to_ascii_uppercase().contains("IMAGE(")).map(str::to_string))
        .filter(|u| gridcraft_ui_egui::formula_graphics::ready(u))
        .collect()
}

fn snapshot(ctx: &egui::Context, p: &Parts) -> Value {
    let others: Vec<Value> = crate::live::others().iter().map(|o| json!({ "username": o.username, "name": o.name })).collect();
    if !crate::account::app_shown() {
        return json!({ "screen": "signin", "account": crate::account::test_state(ctx) });
    }
    if p.home.active {
        return json!({ "screen": "home", "home": p.home.test_state(), "account": crate::account::test_state(ctx), "targets": crate::house::targets(ctx) });
    }
    let Some(doc) = p.app.session.active() else {
        return json!({ "screen": "sheet", "file": null, "mention": crate::notes::mentions::test_state() });
    };
    let wb = &doc.wb;
    let sheets: Vec<&str> = wb.sheets.iter().map(|s| s.name.as_str()).collect();
    let active = doc.selection.active;
    let ranges: Vec<String> = doc.selection.ranges.iter().map(|r| r.a1()).collect();
    let (cell, format) = match wb.active() {
        Some(sheet) => {
            let input = sheet.input_text(active);
            let number = match sheet.value(active) {
                gridcraft_engine::core::Value::Number(n) => Some(n),
                _ => None,
            };
            let st = wb.styles.get(sheet.style_id(active));
            let remark = sheet.comments.get(&active);
            let comment = remark.filter(|c| c.threaded).map(|c| {
                let replies: Vec<Value> = c.replies.iter().map(|(a, t)| json!({ "author": a, "text": t })).collect();
                json!({ "author": c.author, "text": c.text, "replies": replies, "resolved": c.resolved })
            });
            // The cell's data validation rule (kind, list items, drop-down settings).
            let validation = sheet.validations.iter().find(|v| v.ranges.iter().any(|r| r.contains(active))).map(|v| {
                let items = gridcraft_engine::cmd::data::list_items(wb, wb.active_sheet(), active, v);
                let colours: Vec<Value> = v.item_colors.iter().map(|c| json!({ "item": c.item, "color": c.color.resolve(&wb.theme) })).collect();
                json!({
                    "kind": format!("{:?}", v.kind),
                    "items": items,
                    "multiple": v.multiple,
                    "display": format!("{:?}", v.display),
                    "itemColors": colours,
                })
            });
            (
                json!({
                    "value": gridcraft_engine::display::value_text(wb, sheet, active),
                    "number": number,
                    "input": input,
                    "formula": input.starts_with('=').then_some(&input),
                    "note": remark.filter(|c| !c.threaded).map(|c| &c.text),
                    "comment": comment,
                    "validation": validation,
                    // Sheets draws no "number stored as text" marker or warning button (fork 0109): always 0.
                    "numberTextMarkers": 0,
                    "isText": matches!(sheet.value(active), gridcraft_engine::core::Value::Text(_)),
                }),
                json!({
                    "bold": st.font.bold,
                    "italic": st.font.italic,
                    "underline": st.font.underline != Underline::None,
                    "strike": st.font.strike,
                    "font": st.font.name,
                    "size": st.font.size,
                    "numberFormat": &*st.num_fmt.0,
                    "hAlign": format!("{:?}", st.align.h),
                    "wrap": st.align.wrap,
                }),
            )
        }
        None => (Value::Null, Value::Null),
    };
    // The words the save symbol shows (files.rs / live.rs set them; shell.rs falls back the same way).
    let saving = crate::shell::save_status(ctx, doc.is_dirty());
    let linked = p.files.linked().filter(|(uid, _)| *uid == doc.uid);
    json!({
        "screen": "sheet",
        "mention": crate::notes::mentions::test_state(),
        // The Paste standard (crate starbase-clipboard).
        "pastePromptShown": starbase_clipboard::prompt_shown(),
        "ownCopyCurrent": starbase_clipboard::own_copy_current(),
        "targets": crate::house::targets(ctx),
        "file": doc.display_title(),
        "fileId": linked.map(|(_, id)| id),
        "folder": p.files.folder(),
        "sheet": wb.active().map(|s| s.name.as_str()),
        "sheetIndex": wb.active_sheet(),
        "sheets": sheets,
        "active": active.a1(),
        "selection": ranges.join(","),
        "freeze": wb.active().and_then(|s| s.freeze).map(|(r, c)| json!({ "rows": r, "cols": c })),
        // The sheet's grid size (rows × columns); `sized` false = Excel's full grid.
        "grid": wb.active().map(|s| json!({ "rows": s.row_count(), "cols": s.col_count(), "sized": s.grid_rows.is_some() })),
        "filterView": crate::shell::filterviews::state(&*p.app),
        "smartFill": crate::smartfill::suggestion(p.app).map(|s| json!({
            "at": s.at.a1(),
            "cells": s.cells.iter().map(|(c, t)| json!([c.a1(), t])).collect::<Vec<_>>(),
        })),
        "alternating": wb.active().map(crate::altcolours::state),
        "pivot": crate::pivotpanel::state(&*p.app),
        // Test-only: the non-empty cells of the selection as [a1, input], up to 2000, in one call.
        "cells": wb.active().map(|s| selection_cells(s, &doc.selection.ranges)),
        "imagesReady": formula_images_ready(&*p.app),
        "linkCard": crate::linkcard::shown(),
        "external": json!({ "verdict": crate::external::verdict().map(|v| format!("{v:?}")), "held": crate::external::held_count() }),
        "history": p.files.history_state(),
        "cell": cell,
        "format": format,
        "saved": saving.state == crate::shell::SaveState::Saved,
        "saveStatus": saving.text,
        "saveIcon": crate::shell::save_info(ctx, doc.is_dirty()).kind.word(),
        "dirty": doc.is_dirty(),
        "canUndo": !doc.undo.is_empty(),
        "canRedo": !doc.redo.is_empty(),
        "live": crate::live::status_for(doc.uid).is_some(),
        "peers": others.len(),
        "people": others,
        "compact": p.compact,
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

async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}
