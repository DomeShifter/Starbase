//! Insert › Image › "Insert image in cell" / "over cells": opens the browser's file chooser
//! straight away, as Sheets does, instead of GridCraft's picture dialog (to-do row 72). The page
//! (index.html, `__ssPickImage`) reads the chosen file; the app polls for it and inserts it.

use gridcraft_ui_egui::SheetApp;
use serde_json::json;
use wasm_bindgen::{JsCast, JsValue};

/// Asks the page for a picture; `placement` is "cell" or "overCells".
pub fn pick(placement: &str) {
    let Some(w) = web_sys::window() else { return };
    if let Some(f) = js_sys::Reflect::get(&w, &"__ssPickImage".into()).ok().and_then(|f| f.dyn_into::<js_sys::Function>().ok()) {
        let _ = f.call1(&w, &JsValue::from_str(placement));
    }
}

/// Inserts a picture the page has read (base64, type, placement), if any. While the chooser is
/// open the app looks again twice a second: a file chosen in it doesn't wake the app by itself.
pub fn logic(app: &mut SheetApp, ctx: &egui::Context) {
    let Some(w) = web_sys::window() else { return };
    if js_sys::Reflect::get(&w, &"__ssPickingImage".into()).is_ok_and(|v| v.is_truthy()) {
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
    let v = js_sys::Reflect::get(&w, &"__ssPickedImage".into()).unwrap_or(JsValue::NULL);
    if v.is_null() || v.is_undefined() {
        return;
    }
    let _ = js_sys::Reflect::set(&w, &"__ssPickedImage".into(), &JsValue::NULL);
    let arr = js_sys::Array::from(&v);
    let (Some(b64), Some(mime), Some(placement)) = (arr.get(0).as_string(), arr.get(1).as_string(), arr.get(2).as_string()) else { return };
    app.run_or_alert("insert.picture", json!({ "base64": b64, "mime": mime, "placement": placement }));
}
