//! "Scan now" (POST/GET /api/photos/scan): look for new pictures straight away, then new faces.
//! The button sits in the Photos toolbar and on the People page; its words say how far it got.

use std::sync::{Arc, Mutex};

use egui::{RichText, Ui};
use serde_json::{Value as Json, json};

use crate::house;
use crate::store::{Backend, Method};
use crate::theme;

#[derive(Default)]
pub(in crate::home) struct ScanNow {
    /// `{"last": GET reply, "error": text}`, filled by the server's answers.
    st: Arc<Mutex<Json>>,
    /// Started and not yet finished.
    running: bool,
    /// When the state was last asked for (egui time).
    poll: f64,
    /// The words shown last (also for the test hook).
    said: String,
    /// A scan just finished: PhotoMode lists the folder and People again.
    pub finished: bool,
}

impl ScanNow {
    fn ask(&self, ctx: &egui::Context, store: &Backend, m: Method, key: &'static str) {
        let (st, ctx2) = (self.st.clone(), ctx.clone());
        let body = matches!(m, Method::Post).then(|| b"{}".to_vec());
        store.call(
            ctx,
            m,
            "api/photos/scan",
            body,
            Box::new(move |a| {
                let mut s = st.lock().unwrap_or_else(|e| e.into_inner());
                if !s.is_object() {
                    *s = json!({});
                }
                match a {
                    Ok((200, b)) => s[key] = serde_json::from_slice(&b).unwrap_or_default(),
                    Ok((code, b)) => {
                        let v: Json = serde_json::from_slice(&b).unwrap_or_default();
                        s["error"] = json!(v["error"].as_str().map_or(format!("The server said {code}."), str::to_string));
                    }
                    Err(e) => s["error"] = json!(e),
                }
                ctx2.request_repaint();
            }),
        );
    }

    /// The button and its words. True once, when a scan has just finished (to refresh lists).
    pub fn ui(&mut self, ui: &mut Ui, store: &Backend) -> bool {
        let ctx = ui.ctx().clone();
        let st = self.st.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut finished = false;
        if let Some(e) = st["error"].as_str() {
            self.running = false;
            self.said = e.to_string();
        } else if self.running {
            let last = &st["last"];
            let (done, all) = (last["faces"]["done"].as_u64().unwrap_or(0), last["faces"]["all"].as_u64().unwrap_or(0));
            let faces_busy = last["faces_on"] == true && (last["thumbs_waiting"].as_u64().unwrap_or(0) > 0 || done < all);
            if last.is_null() || st["started"].is_null() || last["busy"] != false {
                self.said = "Looking for new photos…".into();
            } else if faces_busy {
                self.said = format!("Finding faces: {done} of {all}");
            } else {
                self.running = false;
                finished = true;
                self.finished = true;
                let (n, f) = (last["photos_new"].as_u64().unwrap_or(0), last["faces_new"].as_u64().unwrap_or(0));
                self.said = match (n, last["faces_on"] == true) {
                    (0, _) => "No new photos".into(),
                    (1, true) => format!("Up to date — 1 new photo, {f} new faces"),
                    (n, true) => format!("Up to date — {n} new photos, {f} new faces"),
                    (1, false) => "Up to date — 1 new photo".into(),
                    (n, false) => format!("Up to date — {n} new photos"),
                };
            }
            let now = ui.input(|i| i.time);
            if !st["started"].is_null() && now - self.poll > 1.5 {
                self.poll = now;
                self.ask(&ctx, store, Method::Get, "last");
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }
        let b = ui.add_enabled_ui(!self.running, |ui| super::shell::pill(ui, Some("refresh"), "Scan now", false)).inner;
        let b = b.on_hover_text("Look for new photos and faces now");
        house::target(&ctx, "photos:scan-now", b.rect);
        if b.clicked() {
            *self.st.lock().unwrap_or_else(|e| e.into_inner()) = json!({});
            self.running = true;
            self.poll = ui.input(|i| i.time);
            self.said = "Looking for new photos…".into();
            self.ask(&ctx, store, Method::Post, "started");
        }
        if !self.said.is_empty() {
            let l = if self.running && self.said.starts_with("Finding faces") {
                let last = &st["last"]["faces"];
                let (done, all) = (last["done"].as_u64().unwrap_or(0), last["all"].as_u64().unwrap_or(1).max(1));
                ui.add(egui::ProgressBar::new(done as f32 / all as f32).desired_width(200.0).text(RichText::new(&self.said).size(house::NOTE)))
            } else {
                ui.label(RichText::new(&self.said).size(house::NOTE).color(theme::text_dim()))
            };
            house::target(&ctx, "photos:scan-status", l.rect);
        }
        finished
    }

    #[cfg(target_arch = "wasm32")]
    pub fn test_state(&self) -> Json {
        let st = self.st.lock().unwrap_or_else(|e| e.into_inner()).clone();
        json!({"running": self.running, "said": self.said, "last": st["last"], "error": st["error"]})
    }
}
