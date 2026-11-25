//! The browser's own Cut / Copy / Paste menu for text being edited (to-do 3 row 14).
//!
//! The page is one canvas, and eframe stops the browser's menu on it (its `contextmenu`
//! listener), so a right-click in a cell being edited (or in the formula bar while editing)
//! could only show a house menu, which can't paste on plain http. Here a right press over that
//! text is taken before eframe sees it (a `pointerdown` listener on the window, capture phase):
//! a hidden text box (`#ss_textmenu`) is put under the pointer with the cell's text and the same
//! selection, and focused, so the `contextmenu` that follows lands on a real text box and the
//! browser shows its own menu for it. Because egui never sees that press, the selection it shows
//! stays as it was (egui moves the caret on any press). What the menu does is copied back:
//! Cut, Paste and Delete change the box's text (an `input` event), Select all its selection.
//! Copy needs nothing (the browser copies from the box). Then the keyboard goes back to the
//! canvas. A key pressed while the box still has it (the menu was closed with Esc) is handed
//! to egui as if typed in the cell.
//!
//! Only on the web: the desktop app keeps the house menu (editmenu.rs).

use std::cell::RefCell;

use gridcraft_ui_egui::SheetApp;

use crate::editmenu::{char_at, key_events, utf16_at};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::Closure;

/// The id of the hidden text box (tests look for it).
pub const PROXY_ID: &str = "ss_textmenu";

/// What the next frame applies to the text being edited.
enum Pending {
    /// The box's text changed (Cut, Paste, Delete, Undo in the menu): the new text and caret.
    Text(String, usize),
    /// The box's selection changed (Select all): anchor and caret, in characters.
    Select(usize, usize),
    /// The keyboard went back to the canvas: the editor takes it again, selection kept.
    Restore,
}

/// Where the text being edited is, for the right press (egui points), and what it holds.
struct Armed {
    rects: Vec<egui::Rect>,
    zoom: f32,
    text: String,
    anchor: usize,
    caret: usize,
}

#[derive(Default)]
struct State {
    armed: Option<Armed>,
    /// The box has the keyboard for the browser's menu.
    active: bool,
    /// The right button went down on the text: its release is ours too.
    pressing: bool,
    /// The box was opened this frame: other menus close (exactly one menu shows, row 22).
    opened: bool,
    pending: Vec<Pending>,
    /// Keys pressed while the box had the keyboard, handed to egui once the editor has taken
    /// the keyboard back (`keys_ready`): in that frame it puts its selection back, which would
    /// undo a key's caret move.
    keys: Vec<egui::Event>,
    keys_ready: bool,
    ctx: Option<egui::Context>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn document() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}

fn proxy() -> Option<web_sys::HtmlTextAreaElement> {
    document()?.get_element_by_id(PROXY_ID)?.dyn_into().ok()
}

fn canvas() -> Option<web_sys::HtmlElement> {
    document()?.get_element_by_id(crate::web::CANVAS_ID)?.dyn_into().ok()
}

fn repaint() {
    STATE.with(|s| {
        if let Some(ctx) = &s.borrow().ctx {
            ctx.request_repaint();
        }
    });
}

/// Where the box waits, off the page. It always takes the pointer (`pointer-events` stays
/// `auto`): in Firefox, an element whose `pointer-events` changed around a right press kept its
/// old place (its `left` and `top` changes were ignored), so the `contextmenu` went to the canvas.
const PARKED: &str = "-10000px";

/// The box moves off the page, so the pointer reaches the canvas again (it keeps the keyboard
/// until `give_back`).
fn park(el: &web_sys::HtmlTextAreaElement) {
    let style = el.style();
    let _ = style.set_property("left", PARKED);
    let _ = style.set_property("top", PARKED);
}

/// The keyboard goes back to the canvas (eframe then gives it to its own text input). With
/// `restore` the editor takes the keyboard back with its selection (after a key or a menu
/// pick); a click on the page leaves that to the click.
fn give_back(restore: bool) {
    let was = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let was = s.active;
        s.active = false;
        if was && restore {
            s.pending.push(Pending::Restore);
        }
        was
    });
    if !was {
        return;
    }
    if let Some(el) = proxy() {
        park(&el);
        let _ = el.blur();
        el.set_value("");
    }
    if let Some(c) = canvas() {
        let o = web_sys::FocusOptions::new();
        o.set_prevent_scroll(true);
        let _ = c.focus_with_options(&o);
    }
    repaint();
}

/// Copies the box's text and caret to the editor on the next frame.
fn take_text(el: &web_sys::HtmlTextAreaElement) {
    let value = el.value();
    let caret = char_at(&value, el.selection_end().ok().flatten().unwrap_or(0));
    STATE.with(|s| s.borrow_mut().pending.push(Pending::Text(value, caret)));
    repaint();
}

/// Listens for `name` on `target` for the life of the page.
fn listen<E: JsCast + 'static>(target: &web_sys::EventTarget, name: &str, capture: bool, mut f: impl FnMut(E) + 'static) {
    let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |e: web_sys::Event| f(e.unchecked_into::<E>()));
    let o = web_sys::AddEventListenerOptions::new();
    o.set_capture(capture);
    let _ = target.add_event_listener_with_callback_and_add_event_listener_options(name, cb.as_ref().unchecked_ref(), &o);
    cb.forget();
}

/// Creates the hidden box and its listeners (once).
pub fn install(ctx: &egui::Context) {
    if STATE.with(|s| s.borrow().ctx.is_some()) {
        return;
    }
    let Some(doc) = document() else { return };
    if doc.get_element_by_id(PROXY_ID).is_some() {
        return;
    }
    let Some(el) = doc.create_element("textarea").ok().and_then(|e| e.dyn_into::<web_sys::HtmlTextAreaElement>().ok()) else {
        return;
    };
    el.set_id(PROXY_ID);
    let _ = el.set_attribute("aria-hidden", "true");
    let _ = el.set_attribute("tabindex", "-1");
    let _ = el.set_attribute("spellcheck", "false");
    let _ = el.set_attribute("autocomplete", "off");
    let style = el.style();
    for (k, v) in [
        ("position", "fixed"),
        ("left", PARKED),
        ("top", PARKED),
        ("width", "24px"),
        ("height", "24px"),
        ("margin", "0"),
        ("padding", "0"),
        ("border", "none"),
        ("outline", "none"),
        ("resize", "none"),
        ("overflow", "hidden"),
        ("opacity", "0"),
        ("color", "transparent"),
        ("background", "transparent"),
        ("caret-color", "transparent"),
        ("font-size", "16px"),
        ("white-space", "pre"),
        ("z-index", "2147483647"),
    ] {
        let _ = style.set_property(k, v);
    }
    if let Some(body) = doc.body() {
        let _ = body.append_child(&el);
    }
    STATE.with(|s| s.borrow_mut().ctx = Some(ctx.clone()));
    let Some(window) = web_sys::window() else { return };

    // A right press on the text being edited: the box takes it, before eframe's canvas does.
    listen(&window, "pointerdown", true, {
        let el = el.clone();
        move |e: web_sys::PointerEvent| {
            let ours = e.button() == 2 && e.pointer_type() != "touch" && arm_at(&el, &e);
            if ours {
                // No mouse events follow (the box keeps the selection we gave it), but the
                // browser still sends `contextmenu`, to the box now under the pointer.
                e.prevent_default();
                e.stop_propagation();
            } else if STATE.with(|s| s.borrow().active) {
                give_back(false);
            }
        }
    });
    // Its release is ours too: eframe would see a release without a press.
    listen(&window, "pointerup", true, |e: web_sys::PointerEvent| {
        if e.button() == 2 && STATE.with(|s| std::mem::take(&mut s.borrow_mut().pressing)) {
            e.stop_propagation();
        }
    });
    listen(&el, "contextmenu", false, {
        let el = el.clone();
        move |e: web_sys::MouseEvent| {
            // The browser's menu: let it show, and let later clicks reach the canvas again.
            e.stop_propagation();
            park(&el);
        }
    });
    listen(&el, "input", false, {
        let el = el.clone();
        move |_: web_sys::Event| {
            if STATE.with(|s| s.borrow().active) {
                take_text(&el);
                give_back(true);
            }
        }
    });
    listen(&el, "select", false, {
        let el = el.clone();
        move |_: web_sys::Event| {
            if !STATE.with(|s| s.borrow().active) {
                return;
            }
            let v = el.value();
            let (a, b) = (el.selection_start().ok().flatten().unwrap_or(0), el.selection_end().ok().flatten().unwrap_or(0));
            let backward = el.selection_direction().ok().flatten().as_deref() == Some("backward");
            let (a, b) = (char_at(&v, a), char_at(&v, b));
            let (anchor, caret) = if backward { (b, a) } else { (a, b) };
            STATE.with(|s| s.borrow_mut().pending.push(Pending::Select(anchor, caret)));
            repaint();
        }
    });
    // What the menu copied (or cut) is StarSheets' last copy, for menu Paste (editmenu.rs).
    for name in ["copy", "cut"] {
        let el = el.clone();
        listen(&el.clone(), name, false, move |_: web_sys::Event| {
            let v = el.value();
            let (a, b) = (el.selection_start().ok().flatten().unwrap_or(0), el.selection_end().ok().flatten().unwrap_or(0));
            let (a, b) = (char_at(&v, a.min(b)), char_at(&v, a.max(b)));
            let picked: String = v.chars().skip(a).take(b - a).collect();
            if !picked.is_empty() {
                crate::editmenu::remember_copy(&picked);
            }
        });
    }
    // A key while the box has the keyboard (the menu was closed): the clipboard keys work on
    // the box as in any text box; anything else goes to the cell.
    listen(&el, "keydown", false, |e: web_sys::KeyboardEvent| {
        if !STATE.with(|s| s.borrow().active) {
            return;
        }
        let k = e.key();
        let command = e.ctrl_key() || e.meta_key();
        if command && matches!(k.to_lowercase().as_str(), "c" | "x" | "v" | "a" | "z" | "y") {
            return;
        }
        if k == "Process" || k == "Dead" || k == "Unidentified" {
            // A composing key: let it reach eframe's own text input.
            give_back(true);
            return;
        }
        e.prevent_default();
        e.stop_propagation();
        let modifiers = egui::Modifiers { alt: e.alt_key(), ctrl: e.ctrl_key(), shift: e.shift_key(), mac_cmd: e.meta_key(), command };
        let events = key_events(&k, modifiers);
        STATE.with(|s| s.borrow_mut().keys.extend(events));
        give_back(true);
    });
    listen(&el, "blur", false, |_: web_sys::FocusEvent| {
        // Another window or tab took the keyboard: nothing more comes from the box.
        if STATE.with(|s| s.borrow().active) {
            give_back(false);
        }
    });
}

/// Puts the box under the pointer with the text and selection being edited, if the press is
/// on that text; true when it did.
fn arm_at(el: &web_sys::HtmlTextAreaElement, e: &web_sys::PointerEvent) -> bool {
    let Some(c) = canvas() else { return false };
    let r = c.get_bounding_client_rect();
    let (x, y) = (e.client_x() as f32, e.client_y() as f32);
    let picked = STATE.with(|s| {
        let s = s.borrow();
        let a = s.armed.as_ref()?;
        let p = egui::pos2((x - r.left() as f32) / a.zoom, (y - r.top() as f32) / a.zoom);
        a.rects.iter().any(|rect| rect.contains(p)).then(|| (a.text.clone(), a.anchor, a.caret))
    });
    let Some((text, anchor, caret)) = picked else { return false };
    el.set_value(&text);
    let style = el.style();
    let _ = style.set_property("left", &format!("{}px", x - 12.0));
    let _ = style.set_property("top", &format!("{}px", y - 12.0));
    let o = web_sys::FocusOptions::new();
    o.set_prevent_scroll(true);
    let _ = el.focus_with_options(&o);
    let (a, b) = (utf16_at(&text, anchor.min(caret)), utf16_at(&text, anchor.max(caret)));
    let _ = el.set_selection_range_with_direction(a, b, if caret < anchor { "backward" } else { "forward" });
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.active = true;
        s.pressing = true;
        s.opened = true;
    });
    repaint();
    true
}

/// Hands the keys the box caught to egui (call from the raw input hook).
pub fn raw_input(raw: &mut egui::RawInput) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if std::mem::take(&mut s.keys_ready) {
            raw.events.append(&mut s.keys);
        }
    });
}

/// Call before the grid: applies what the browser's menu did to the text being edited. True
/// when the menu opened since last frame (other menus close: one menu at a time).
pub fn before(app: &mut SheetApp) -> bool {
    let (pending, opened, active) = STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.pending.is_empty() && !s.keys.is_empty() && !s.active {
            s.keys_ready = true;
            if let Some(ctx) = &s.ctx {
                ctx.request_repaint();
            }
        }
        (std::mem::take(&mut s.pending), std::mem::take(&mut s.opened), s.active)
    });
    if let Some(e) = app.editor.as_mut() {
        for p in pending {
            match p {
                Pending::Text(text, caret) => {
                    e.text = text;
                    e.caret = caret;
                    e.anchor = caret;
                    e.sync_caret = true;
                    e.request_focus = true;
                }
                Pending::Select(anchor, caret) => {
                    e.anchor = anchor;
                    e.caret = caret;
                    e.keep_selection = true;
                    e.request_focus = true;
                }
                Pending::Restore => {
                    e.keep_selection = !e.sync_caret;
                    e.request_focus = true;
                }
            }
        }
    } else if active {
        // The edit ended (another user's change, a reload): the box lets go.
        give_back(false);
    }
    opened
}

/// Call after the grid: where the text being edited is now, for the next right press.
pub fn after(app: &SheetApp, ctx: &egui::Context) {
    let armed = app.editor.as_ref().map(|e| {
        let cell = ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::unique("gridcraft.cell_editor.rect")));
        let bar = ctx.read_response(egui::Id::unique("gridcraft.formula_bar")).map(|r| r.rect);
        Armed { rects: cell.into_iter().chain(bar).collect(), zoom: ctx.zoom_factor(), text: e.text.clone(), anchor: e.anchor, caret: e.caret }
    });
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        // While the box has the keyboard, keep what it was given.
        if !s.active {
            s.armed = armed;
        }
    });
}
