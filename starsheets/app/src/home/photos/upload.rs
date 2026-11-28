//! Photos > Upload: pictures and videos from this device (on a phone the browser offers the camera
//! roll and the camera) saved into the folder shown. Two at a time, each with its own status; a
//! failed one is tried again a few times and then waits for "Retry"; a file already in the folder
//! with exactly the same bytes is reported as a duplicate and not saved again (the server's
//! `skip_dup=1`, crates/server/src/store.rs).

use std::rc::Rc;

use egui::{RichText, Ui};
use serde_json::Value as Json;
#[cfg(target_arch = "wasm32")]
use serde_json::json;

use super::{Got, PhotoMode};
use crate::house;
use crate::store::{Backend, Method, enc};
use crate::theme;

const AT_ONCE: usize = 2;
const TRIES: u32 = 3;

#[derive(Clone, Debug, PartialEq)]
pub(in crate::home) enum State {
    Waiting,
    Sending,
    Done,
    Duplicate,
    Failed(String),
}

#[derive(Debug)]
pub(in crate::home) struct Up {
    pub name: String,
    pub folder: String,
    /// Kept until it is saved, for a retry.
    bytes: Option<Rc<Vec<u8>>>,
    pub state: State,
    tries: u32,
    retry_at: f64,
}

#[derive(Default)]
pub(in crate::home) struct Uploads {
    pub files: Vec<Up>,
    /// Something was saved since the grid was last asked for.
    dirty: bool,
    /// Where the Upload button was drawn (top-left, test hook).
    // Only the web test hook reads it.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub btn: Option<[f32; 2]>,
}

impl Uploads {
    fn count(&self, f: impl Fn(&State) -> bool) -> usize {
        self.files.iter().filter(|u| f(&u.state)).count()
    }
    pub fn busy(&self) -> bool {
        self.count(|s| matches!(s, State::Waiting | State::Sending)) > 0
    }
    #[cfg(target_arch = "wasm32")]
    pub fn test_state(&self) -> Json {
        let name = |s: &State| match s {
            State::Waiting => "waiting".to_string(),
            State::Sending => "sending".to_string(),
            State::Done => "done".to_string(),
            State::Duplicate => "duplicate".to_string(),
            State::Failed(e) => format!("failed: {e}"),
        };
        json!({
            "button": self.btn, "total": self.files.len(), "busy": self.busy(),
            "files": self.files.iter().map(|u| json!({"name": u.name, "state": name(&u.state), "tries": u.tries})).collect::<Vec<_>>(),
        })
    }
}

impl PhotoMode {
    /// Opens the device's picker (pictures and videos).
    pub(super) fn upload_pick(&self, ctx: &egui::Context, store: &Backend) {
        let (inbox, ctx) = (self.inbox.clone(), ctx.clone());
        store.pick_media(Box::new(move |name, bytes| PhotoMode::push(&inbox, &ctx, Got::Picked(name, bytes))));
    }

    /// A file came from the picker: line it up.
    pub(in crate::home) fn upload_add(&mut self, folder: &str, name: String, bytes: Vec<u8>) {
        self.up.files.push(Up { name, folder: folder.to_string(), bytes: Some(Rc::new(bytes)), state: State::Waiting, tries: 0, retry_at: 0.0 });
    }

    /// One answer for file `i`.
    pub(super) fn upload_answer(&mut self, i: usize, r: Result<bool, String>, now: f64) {
        let Some(u) = self.up.files.get_mut(i) else { return };
        match r {
            Ok(dup) => {
                u.state = if dup { State::Duplicate } else { State::Done };
                u.bytes = None;
                self.up.dirty |= !dup;
            }
            Err(_) if u.tries < TRIES => {
                u.state = State::Waiting;
                u.retry_at = now + 2.0 * u.tries as f64;
            }
            Err(e) => u.state = State::Failed(e),
        }
    }

    /// Starts what can start; asks for the grid again when the line is empty.
    pub(super) fn upload_pump(&mut self, ctx: &egui::Context, store: &Backend, folder: Option<&str>, now: f64) {
        let mut sending = self.up.count(|s| *s == State::Sending);
        for i in 0..self.up.files.len() {
            if sending >= AT_ONCE {
                break;
            }
            let u = &mut self.up.files[i];
            if u.state != State::Waiting || u.retry_at > now {
                continue;
            }
            let Some(bytes) = u.bytes.clone() else { continue };
            u.state = State::Sending;
            u.tries += 1;
            sending += 1;
            let url = format!("api/files?name={}&folder={}&as_is=1&skip_dup=1", enc(&u.name), enc(&u.folder));
            let (inbox, c2) = (self.inbox.clone(), ctx.clone());
            store.call(
                ctx,
                Method::Post,
                &url,
                Some((*bytes).clone()),
                Box::new(move |a| {
                    let r = match a {
                        Ok((200, b)) if serde_json::from_slice::<Json>(&b).is_ok_and(|v| v["duplicate"] == true) => Ok(true),
                        Ok((200 | 201, _)) => Ok(false),
                        Ok((c, _)) => Err(format!("the server said {c}")),
                        Err(e) => Err(e),
                    };
                    PhotoMode::push(&inbox, &c2, Got::Uploaded(i, r));
                }),
            );
        }
        if self.up.files.iter().any(|u| u.state == State::Waiting && u.retry_at > now) {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
        if self.up.dirty && !self.up.busy() {
            self.up.dirty = false;
            if let Some(f) = folder {
                self.list(ctx, store, f);
            }
        }
    }

    /// The upload list under the bar: one bar for all, one line per file.
    pub(super) fn upload_ui(&mut self, ui: &mut Ui) {
        if self.up.files.is_empty() {
            return;
        }
        let total = self.up.files.len();
        let settled = self.up.count(|s| matches!(s, State::Done | State::Duplicate | State::Failed(_)));
        ui.add_space(house::S);
        let mut retry = None;
        let mut clear = false;
        house::card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            let (new, dup) = (self.up.count(|s| *s == State::Done), self.up.count(|s| *s == State::Duplicate));
            let bad = self.up.count(|s| matches!(s, State::Failed(_)));
            let mut head = if self.up.busy() { format!("Uploading {} of {total}", settled + 1) } else { format!("Uploaded {new} of {total}") };
            if dup > 0 {
                head.push_str(&format!(" · {dup} already here"));
            }
            if bad > 0 {
                head.push_str(&format!(" · {bad} failed"));
            }
            ui.label(RichText::new(head).size(house::BODY).color(theme::text()));
            ui.add(egui::ProgressBar::new(settled as f32 / total as f32).desired_height(8.0).fill(super::super::ACCENT));
            ui.add_space(house::S);
            for (i, u) in self.up.files.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    let (text, colour) = match &u.state {
                        State::Waiting => ("waiting".to_string(), theme::text_dim()),
                        State::Sending => ("uploading…".to_string(), theme::text_dim()),
                        State::Done => ("uploaded".to_string(), theme::text()),
                        State::Duplicate => ("already here, skipped".to_string(), theme::text_dim()),
                        State::Failed(e) => (format!("failed ({e})"), house::ERROR),
                    };
                    ui.label(RichText::new(&u.name).size(house::NOTE).color(theme::text()));
                    ui.label(RichText::new(text).size(house::NOTE).color(colour));
                    if matches!(u.state, State::Failed(_)) && ui.add(house::link("Retry")).clicked() {
                        retry = Some(i);
                    }
                });
            }
            if !self.up.busy() && ui.add(house::link("Clear")).clicked() {
                clear = true;
            }
        });
        if let Some(i) = retry {
            let u = &mut self.up.files[i];
            u.state = State::Waiting;
            u.tries = 0;
            u.retry_at = 0.0;
        }
        if clear {
            self.up.files.retain(|u| matches!(u.state, State::Failed(_)));
        }
    }
}
