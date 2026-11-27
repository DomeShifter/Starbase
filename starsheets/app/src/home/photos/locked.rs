//! Photos > Locked: pictures hidden from the library, albums, search and shares. Opening it asks
//! for the account password again (`POST /api/photos/locked/unlock`); it closes by itself after
//! 5 minutes without use, or when you leave it. The server side is
//! `crates/server/src/photos/locked.rs`. Nothing is encrypted on disk: pictures are hidden.
//!
//! TODO: also ask the two-step code once two-step sign-in exists (none on main yet).

use egui::{RichText, Ui};
use serde_json::{Value as Json, json};

use super::albums::{Inbox, Tab, get};
use super::{Got, PhotoMode};
use crate::house;
use crate::store::{Backend, Method};
use crate::theme;

/// Seconds without use before Locked closes (the server also relocks after 5 minutes).
const IDLE: f64 = 300.0;

#[derive(Default)]
pub(in crate::home) struct Locked {
    /// Open (the password was accepted and not yet relocked).
    pub unlocked: bool,
    /// The server was asked whether it is open.
    checked: bool,
    pub pw: String,
    pub error: Option<String>,
    pub busy: bool,
    last_use: f64,
    /// "Move to Locked folder": the pictures waiting for the confirm dialog.
    pub ask: Option<Vec<String>>,
    /// "Move out" asked from the Info panel, done on the next frame.
    pub out_req: Vec<String>,
    /// Left the Locked tab while open: the server is told to relock on the next frame.
    pub lock_req: bool,
}

/// What the Locked tab says while empty: how to put pictures in.
pub(super) const LOCKED_EMPTY: &str = "Nothing here yet. To lock a picture: right-click it in Photos, then Move to Locked folder.";

impl Locked {
    pub fn reset(&mut self) {
        self.lock_req |= self.unlocked;
        self.unlocked = false;
        self.checked = false;
        self.busy = false;
        self.pw.clear();
        self.error = None;
    }
}

fn call(ctx: &egui::Context, store: &Backend, inbox: &Inbox, url: &str, body: Option<Json>, wrap: impl FnOnce(u16, Json) -> Got + 'static) {
    let (inbox, ctx2) = (inbox.clone(), ctx.clone());
    store.call(
        ctx,
        Method::Post,
        url,
        Some(body.unwrap_or_else(|| json!({})).to_string().into_bytes()),
        Box::new(move |a| {
            let g = match a {
                Ok((s, b)) => wrap(s, serde_json::from_slice(&b).unwrap_or_default()),
                Err(e) => Got::Error(e),
            };
            PhotoMode::push(&inbox, &ctx2, g);
        }),
    );
}

impl PhotoMode {
    pub(in crate::home) fn locked_tab(&self) -> bool {
        self.alb.tab == Tab::Locked
    }

    /// The server's answer to "is Locked open", with the pictures when it is.
    pub(super) fn take_locked(&mut self, ctx: &egui::Context, v: &Json) {
        if !self.locked_tab() {
            return;
        }
        self.loading = false;
        self.lk.unlocked = v["unlocked"].as_bool().unwrap_or(false);
        if self.lk.unlocked {
            self.lk.last_use = ctx.input(|i| i.time);
            self.waiting = v["waiting"].as_u64().unwrap_or(0);
            let pics = Self::parse_pics(v);
            self.alb.selected.retain(|id| pics.iter().any(|p| &p.id == id));
            self.set_pics(pics);
        } else {
            self.pics.clear();
        }
    }

    fn ask_locked(&mut self, ctx: &egui::Context, store: &Backend) {
        self.lk.checked = true;
        self.loading = true;
        get(ctx, store, &self.inbox, "api/photos/locked", Got::LockedState);
    }

    /// The typed password goes to the server.
    pub(super) fn unlock(&mut self, ctx: &egui::Context, store: &Backend) {
        if self.lk.busy || self.lk.pw.is_empty() {
            return;
        }
        self.lk.busy = true;
        self.lk.error = None;
        let pw = std::mem::take(&mut self.lk.pw);
        call(ctx, store, &self.inbox, "api/photos/locked/unlock", Some(json!({ "password": pw })), |s, v| {
            if s == 200 {
                Got::Unlocked
            } else {
                Got::Wrong(v["error"].as_str().map_or_else(|| "That password is not right.".to_string(), str::to_string))
            }
        });
    }

    /// The server accepted the password: the pictures are asked for.
    pub(super) fn unlocked(&mut self, ctx: &egui::Context, store: &Backend) {
        self.lk.busy = false;
        self.lk.error = None;
        if self.locked_tab() {
            self.ask_locked(ctx, store);
        }
    }

    /// Closes Locked now: the server forgets the unlock, the screen asks for the password again.
    pub(super) fn relock(&mut self, ctx: &egui::Context, store: &Backend) {
        if self.lk.unlocked {
            call(ctx, store, &self.inbox, "api/photos/locked/lock", None, |_, _| Got::Nothing);
        }
        self.lk.reset();
        self.lk.lock_req = false;
        self.lk.checked = true;
        self.loading = false;
        self.pics.clear();
        self.viewer = None;
        self.clear_selection();
    }

    /// Moves `ids` into (`into`) or out of Locked; the answer takes them off the screen.
    pub(super) fn move_locked(&mut self, ctx: &egui::Context, store: &Backend, ids: Vec<String>, into: bool) {
        let url = if into { "api/photos/locked/add" } else { "api/photos/locked/remove" };
        let shown = ids.clone();
        call(ctx, store, &self.inbox, url, Some(json!({ "files": ids })), move |s, v| {
            if s == 200 { Got::Moved(shown, into) } else { Got::Error(crate::home::photos::said(&v, s)) }
        });
    }

    /// The server moved pictures: they leave this list, and the note says so.
    pub(super) fn moved(&mut self, ctx: &egui::Context, store: &Backend, ids: &[String], into: bool) {
        self.pics.retain(|p| !ids.contains(&p.id));
        if let Some(v) = &mut self.viewer {
            v.fit_list(self.pics.len());
        }
        self.clear_selection();
        let n = ids.len();
        let what = if n == 1 { "1 picture".to_string() } else { format!("{n} pictures") };
        self.say(ctx, if into { format!("Moved {what} to the Locked folder") } else { format!("Moved {what} out of the Locked folder") }, None);
        self.alb.changed(ctx, store, &self.inbox);
    }

    /// Each frame: the idle timer, leaving Locked, and "Move out" asked in the viewer.
    pub(super) fn locked_logic(&mut self, ctx: &egui::Context, store: &Backend) {
        if !self.lk.out_req.is_empty() {
            let ids = std::mem::take(&mut self.lk.out_req);
            self.move_locked(ctx, store, ids, false);
        }
        if self.lk.lock_req {
            self.lk.lock_req = false;
            call(ctx, store, &self.inbox, "api/photos/locked/lock", None, |_, _| Got::Nothing);
        }
        if !self.locked_tab() {
            if self.lk.unlocked {
                self.relock(ctx, store);
            }
            return;
        }
        if !self.lk.unlocked {
            return;
        }
        let now = ctx.input(|i| i.time);
        let used = ctx.input(|i| !i.events.is_empty() || i.pointer.delta() != egui::Vec2::ZERO || i.smooth_scroll_delta != egui::Vec2::ZERO);
        if used {
            self.lk.last_use = now;
        }
        if now - self.lk.last_use > IDLE {
            self.relock(ctx, store);
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs(5));
        }
    }

    /// The Locked tab: the password screen, or the grid of locked pictures.
    pub(super) fn locked_ui(&mut self, ui: &mut Ui, store: &Backend) {
        let ctx = ui.ctx().clone();
        if !self.lk.checked {
            self.ask_locked(&ctx, store);
        }
        if self.lk.unlocked {
            ui.add_space(house::S);
            house::note(ui, "Locked folder: hidden from the library, albums, search and shares. It closes after 5 minutes unused.");
            if self.alb.selected.is_empty() {
                if ui.add(house::secondary("Lock now")).clicked() {
                    self.relock(&ctx, store);
                    return;
                }
            } else {
                self.sel_bar_ui(ui, store);
            }
            ui.add_space(house::S);
            if self.pics.is_empty() {
                ui.add_space(house::XL);
                let text = if self.loading { "Loading…" } else { LOCKED_EMPTY };
                ui.label(RichText::new(text).size(house::BODY).color(theme::text_dim()));
                return;
            }
            self.grid(ui, store);
            return;
        }
        ui.add_space(house::XL);
        if self.loading && !self.lk.busy {
            ui.label(RichText::new("Loading…").size(house::BODY).color(theme::text_dim()));
            return;
        }
        ui.vertical(|ui| {
            ui.set_max_width(320.0);
            ui.label(RichText::new("Locked folder").size(house::BODY).strong().color(theme::text()));
            ui.add_space(house::S);
            house::note(ui, "Type your password to see the pictures you locked.");
            ui.add_space(house::L);
            let r = ui.add(house::field(&mut self.lk.pw, house::BODY).password(true).hint_text("Password").desired_width(f32::INFINITY));
            house::target(&ctx, "photos:locked:password", r.rect);
            // The password field drops focus on Enter. Consume that press before Drive's
            // keyboard handler can open its previously selected file in the background.
            let enter = r.lost_focus() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            if let Some(e) = &self.lk.error {
                ui.add_space(house::S);
                ui.label(RichText::new(e).size(house::NOTE).color(house::ERROR));
            }
            ui.add_space(house::L);
            let ok = !self.lk.pw.is_empty() && !self.lk.busy;
            let b = if ok { house::primary("Unlock") } else { house::primary_off("Unlock") };
            if (ui.add(b).clicked() || enter) && ok {
                self.unlock(&ctx, store);
            }
        });
    }

    /// The question before pictures go to Locked.
    pub(super) fn locked_dialog(&mut self, ctx: &egui::Context, store: &Backend) {
        let Some(ids) = self.lk.ask.clone() else { return };
        let (mut close, mut yes, mut cancel) = (false, false, false);
        house::dialog(ctx, "photos_lock_confirm", 340.0, "Move to Locked folder?", Some(&mut close), |ui| {
            house::note(ui, "They will be hidden from your library, albums, search and shares. You open them again with your password.");
            ui.add_space(house::L);
            ui.horizontal(|ui| {
                yes = ui.add(house::primary("Move")).clicked();
                if ui.add(house::secondary("Cancel")).clicked() {
                    cancel = true;
                }
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        if yes {
            self.lk.ask = None;
            self.move_locked(ctx, store, ids, true);
        } else if close || cancel {
            self.lk.ask = None;
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl PhotoMode {
    pub(super) fn locked_state(&self) -> Json {
        json!({
            "unlocked": self.lk.unlocked, "checked": self.lk.checked, "busy": self.lk.busy,
            "error": self.lk.error, "asking": self.lk.ask, "count": if self.locked_tab() { self.pics.len() } else { 0 },
        })
    }

    /// `home:photos:locked:unlock {password}`, `:lock`, `:confirm {yes}` (the Move to Locked
    /// question), `:idle` (pretend 5 minutes passed). Open the tab with `home:photos:tab {tab: locked}`.
    pub(in crate::home) fn locked_test(&mut self, ctx: &egui::Context, store: &Backend, id: &str, p: &Json) -> Result<(), String> {
        match id {
            "home:photos:locked:unlock" => {
                self.lk.pw = p.get("password").and_then(Json::as_str).unwrap_or("").to_string();
                self.unlock(ctx, store);
            }
            "home:photos:locked:lock" => self.relock(ctx, store),
            "home:photos:locked:idle" => self.lk.last_use = ctx.input(|i| i.time) - IDLE - 1.0,
            "home:photos:locked:confirm" => {
                let ids = self.lk.ask.take().ok_or("no Move to Locked question is open")?;
                if p.get("yes").and_then(Json::as_bool).unwrap_or(true) {
                    self.move_locked(ctx, store, ids, true);
                }
            }
            _ => return Err(format!("unknown {id}")),
        }
        Ok(())
    }
}
