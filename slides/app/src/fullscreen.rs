//! A slideshow fills the screen, as in Google Slides. DeckCraft asks its window to go full screen
//! (`ViewportCommand::Fullscreen`), which the browser build can't do; here the page asks the
//! browser instead, and leaves full screen when the show ends.
//!
//! Firefox only allows full screen from inside the click or key press itself (Chrome also a few
//! seconds after), never from a later animation frame, which is where egui runs our code. So the
//! page listens for the gestures that start a show (a click on the Slideshow button, whose place
//! the window publishes, `shell::slideshow_rect_id`; F5, Shift+F5 and Ctrl/Cmd+Enter) and asks for
//! full screen in that same event; the show starts on the next frame. A show started otherwise
//! (a menu, a script) runs in the tab.
//!
//! Esc is the browser's own key for leaving full screen and may never reach the page, so leaving
//! full screen while a show runs ends the show.

use std::cell::Cell;
use std::rc::Rc;

use deckcraft_ui_egui::SlideApp;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::closure::Closure;

#[derive(Default)]
pub struct Fullscreen {
    /// A show was running last frame.
    showing: bool,
    /// Full screen was granted during this show.
    granted: bool,
    /// What the event listeners read: a show is running, and the Slideshow button's place.
    shared: Rc<Shared>,
}

#[derive(Default)]
struct Shared {
    showing: Cell<bool>,
    button: Cell<Option<egui::Rect>>,
    /// When a gesture last asked for full screen (ms): if no show follows, full screen is left.
    asked: Cell<Option<f64>>,
    /// The show command that gesture starts, run on the next frame: entering full screen can cost
    /// egui the click or key itself (seen in Firefox), so the page starts the show.
    start: Cell<Option<&'static str>>,
}

fn document() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}

/// Whether the page is full screen now.
pub fn is_on() -> bool {
    document().and_then(|d| d.fullscreen_element()).is_some()
}

/// Calls `name` on `target` (a promise-returning full screen method); a refusal is logged, not
/// left as an unhandled rejection.
fn call(target: &wasm_bindgen::JsValue, name: &str) {
    let Ok(f) = js_sys::Reflect::get(target, &name.into()).and_then(|f| f.dyn_into::<js_sys::Function>()) else { return };
    match f.call0(target) {
        Ok(p) => {
            if let Ok(p) = p.dyn_into::<js_sys::Promise>() {
                let caught = Closure::<dyn FnMut(wasm_bindgen::JsValue)>::new(move |e: wasm_bindgen::JsValue| log::info!("full screen: {e:?}"));
                let _ = p.catch(&caught);
                caught.forget();
            }
        }
        Err(e) => log::info!("full screen: {e:?}"),
    }
}

fn request(shared: &Shared) {
    shared.asked.set(Some(js_sys::Date::now()));
    if !is_on()
        && let Some(el) = document().and_then(|d| d.document_element())
    {
        call(&el, "requestFullscreen");
    }
}

/// The show a key press starts, as DeckCraft's shortcuts: F5 and Ctrl/Cmd+Shift+Enter from the
/// start, Shift+F5 and Ctrl/Cmd+Enter from the current slide.
fn show_for_key(e: &web_sys::KeyboardEvent) -> Option<&'static str> {
    let ctrl = e.ctrl_key() || e.meta_key();
    match (e.key().as_str(), e.shift_key(), ctrl) {
        ("F5", false, _) | ("Enter", true, true) => Some("show.fromStart"),
        ("F5", true, _) | ("Enter", false, true) => Some("show.fromCurrent"),
        _ => None,
    }
}

impl Fullscreen {
    /// Listens (before egui sees them) for the gestures that start a show.
    pub fn new() -> Self {
        let me = Self::default();
        let Some(window) = web_sys::window() else { return me };
        let shared = me.shared.clone();
        let click = Closure::<dyn FnMut(web_sys::PointerEvent)>::new(move |e: web_sys::PointerEvent| {
            let at = egui::pos2(e.client_x() as f32, e.client_y() as f32);
            if !shared.showing.get() && shared.button.get().is_some_and(|r| r.contains(at)) {
                request(&shared);
                shared.start.set(Some("show.fromCurrent"));
            }
        });
        let shared = me.shared.clone();
        let key = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |e: web_sys::KeyboardEvent| {
            if !shared.showing.get()
                && let Some(show) = show_for_key(&e)
            {
                // Ours alone: egui must not start it a second time (and F5 must not reload).
                e.prevent_default();
                e.stop_immediate_propagation();
                request(&shared);
                shared.start.set(Some(show));
            }
        });
        let opts = web_sys::AddEventListenerOptions::new();
        opts.set_capture(true);
        for (name, f) in [("pointerup", click.as_ref().unchecked_ref()), ("keydown", key.as_ref().unchecked_ref())] {
            if let Err(e) = window.add_event_listener_with_callback_and_add_event_listener_options(name, f, &opts) {
                log::warn!("full screen: no {name} listener: {e:?}");
            }
        }
        click.forget();
        key.forget();
        me
    }

    /// Every frame, after the window and DeckCraft have drawn: where the Slideshow button is, and
    /// leaving full screen with the show.
    pub fn tick(&mut self, app: &mut SlideApp, ctx: &egui::Context) {
        // Not in the frame that carries the click: a show started there would take the same click
        // as its first "next slide" (and end a one-slide show at once).
        let clicking = ctx.input(|i| i.pointer.any_pressed() || i.pointer.any_released());
        if !clicking
            && let Some(show) = self.shared.start.take()
            && app.show.is_none()
        {
            let _ = app.run(show, serde_json::json!({}));
        }
        let showing = app.show.is_some();
        let on = is_on();
        self.shared.button.set(if showing { None } else { ctx.data(|d| d.get_temp::<egui::Rect>(crate::shell::slideshow_rect_id())) });
        if showing && on {
            self.granted = true;
        }
        if showing && self.granted && !on {
            // Left full screen (Esc, the browser's own key): the show ends too.
            let _ = app.run("show.end", serde_json::json!({}));
        }
        let showing = app.show.is_some();
        if showing {
            self.shared.asked.set(None);
        } else if on && self.shared.asked.get().is_some_and(|t| js_sys::Date::now() - t > 1500.0) {
            // Asked for with a show's key, but no show started (the key did something else).
            self.shared.asked.set(None);
            if let Some(d) = document() {
                call(&d, "exitFullscreen");
            }
        }
        if !showing && self.showing {
            if on && let Some(d) = document() {
                call(&d, "exitFullscreen");
            }
            self.granted = false;
        }
        self.showing = showing;
        self.shared.showing.set(showing);
    }
}
