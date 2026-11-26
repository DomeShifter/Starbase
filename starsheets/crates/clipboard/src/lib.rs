//! The Paste standard of every Starbase app (docs/decisions.md 2026-11-26, house-style.md
//! "Paste"). A menu Paste (Edit › Paste, a right-click Paste) has to paste what Ctrl+V would,
//! but a web page may only read the clipboard in some cases. So:
//!
//! 1. The app's own last copy counts only while the tab kept the focus: a blur or the tab
//!    being hidden forgets it ([`watch_focus`]), as something else may have been copied meanwhile.
//! 2. Own copy still current: menu Paste pastes it directly ([`PasteRoute::Own`]).
//! 3. Otherwise on https: read the clipboard (the browser may ask) ([`PasteRoute::Read`]).
//! 4. On http, or when the read was refused or empty: show the small "Press Ctrl+V now"
//!    prompt ([`PasteRoute::Prompt`]); it waits, and the Ctrl+V pastes at the place the menu
//!    was opened for. Its reason line can be switched off per browser ([`set_explain`]).

use std::cell::RefCell;

/// The prompt's title.
pub const PROMPT_TITLE: &str = "Press Ctrl+V now";
/// The prompt's one reason line (hidden after "Don't explain again").
pub const PROMPT_REASON: &str = "Browsers only let a page read the clipboard when you press Ctrl+V.";
/// The prompt's tick box.
pub const DONT_EXPLAIN: &str = "Don't explain again";
/// Where the browser remembers "Don't explain again" (localStorage).
pub const EXPLAIN_KEY: &str = "starbase.paste.explain";

/// What a menu Paste does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasteRoute {
    /// Paste the app's own copy at once.
    Own(String),
    /// Ask the browser for the clipboard (https only); on refusal fall back to [`PasteRoute::Prompt`].
    Read,
    /// Show "Press Ctrl+V now" and let the key paste.
    Prompt,
}

/// The decision of a menu Paste. `own`: the app's own copy if still current; `can_read`: the
/// page may try to read the clipboard (secure context; always true on the desktop app);
/// `read_failed`: a read was tried for this paste and gave nothing (refused or empty).
pub fn decide(own: Option<&str>, can_read: bool, read_failed: bool) -> PasteRoute {
    match own.filter(|t| !t.is_empty()) {
        Some(t) => PasteRoute::Own(t.to_string()),
        None if can_read && !read_failed => PasteRoute::Read,
        None => PasteRoute::Prompt,
    }
}

thread_local! {
    static OWN: RefCell<Option<String>> = const { RefCell::new(None) };
    static PROMPT: RefCell<bool> = const { RefCell::new(false) };
    #[cfg(not(target_arch = "wasm32"))]
    static EXPLAIN: RefCell<bool> = const { RefCell::new(true) };
}

/// Remembers text the app just copied (any route: Ctrl+C, a menu Copy, Cut).
pub fn remember(text: &str) {
    OWN.with(|c| *c.borrow_mut() = Some(text.replace("\r\n", "\n")));
}

/// The app's own copy, if the tab hasn't lost the focus since.
pub fn own_copy() -> Option<String> {
    OWN.with(|c| c.borrow().clone()).filter(|t| !t.is_empty())
}

/// For the test hook: `ownCopyCurrent`.
pub fn own_copy_current() -> bool {
    own_copy().is_some()
}

/// Forgets the app's own copy (the tab lost the focus or was hidden).
pub fn forget() {
    OWN.with(|c| *c.borrow_mut() = None);
}

/// Shows the "Press Ctrl+V now" prompt.
pub fn show_prompt() {
    PROMPT.with(|p| *p.borrow_mut() = true);
}

/// For the app's drawing and the test hook (`pastePromptShown`).
pub fn prompt_shown() -> bool {
    PROMPT.with(|p| *p.borrow())
}

/// Closes the prompt.
pub fn close_prompt() {
    PROMPT.with(|p| *p.borrow_mut() = false);
}

/// One frame of the waiting prompt: `pasted` is whether this frame brought a paste (Ctrl+V),
/// `cancelled` whether Esc or a click outside it. Either closes it.
pub fn prompt_frame(pasted: bool, cancelled: bool) {
    if prompt_shown() && (pasted || cancelled) {
        close_prompt();
    }
}

/// Whether the prompt shows its reason line (false after "Don't explain again", per browser).
pub fn explain() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        storage().and_then(|s| s.get_item(EXPLAIN_KEY).ok().flatten()).is_none_or(|v| v != "off")
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        EXPLAIN.with(|e| *e.borrow())
    }
}

/// Sets whether the prompt explains itself (remembered in this browser).
pub fn set_explain(on: bool) {
    #[cfg(target_arch = "wasm32")]
    if let Some(s) = storage() {
        let _ = if on { s.remove_item(EXPLAIN_KEY) } else { s.set_item(EXPLAIN_KEY, "off") };
    }
    #[cfg(not(target_arch = "wasm32"))]
    EXPLAIN.with(|e| *e.borrow_mut() = on);
}

#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// Whether this page may try to read the clipboard (a secure context: https or localhost).
pub fn can_read() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window().is_some_and(|w| w.is_secure_context())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        true
    }
}

/// The desktop app's direct read of the system clipboard (None when empty or unavailable).
#[cfg(not(target_arch = "wasm32"))]
pub fn read_native() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok().filter(|t| !t.is_empty())
}

/// Forgets the own copy whenever the window loses the focus or the tab is hidden (call once at
/// start). Checking `document.hasFocus()` once per frame wasn't enough: egui draws no frame
/// while the tab is in the background and has the focus back by the next one, so a copy made
/// in another app went unnoticed and menu Paste pasted the old own copy.
#[cfg(target_arch = "wasm32")]
pub fn watch_focus() {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    let Some(w) = web_sys::window() else { return };
    let on_blur = Closure::<dyn FnMut(web_sys::Event)>::new(|_: web_sys::Event| forget());
    let _ = w.add_event_listener_with_callback("blur", on_blur.as_ref().unchecked_ref());
    on_blur.forget();
    if let Some(d) = w.document() {
        let doc = d.clone();
        let on_vis = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
            if doc.visibility_state() == web_sys::VisibilityState::Hidden {
                forget();
            }
        });
        let _ = d.add_event_listener_with_callback("visibilitychange", on_vis.as_ref().unchecked_ref());
        on_vis.forget();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_copy_pastes_directly() {
        assert_eq!(decide(Some("a\tb"), false, false), PasteRoute::Own("a\tb".into()));
        assert_eq!(decide(Some("x"), true, false), PasteRoute::Own("x".into()));
    }

    #[test]
    fn without_own_copy_https_reads_and_http_prompts() {
        assert_eq!(decide(None, true, false), PasteRoute::Read);
        assert_eq!(decide(Some(""), true, false), PasteRoute::Read);
        assert_eq!(decide(None, false, false), PasteRoute::Prompt);
        // A refused (or empty) read: the prompt.
        assert_eq!(decide(None, true, true), PasteRoute::Prompt);
    }

    #[test]
    fn focus_loss_forgets_the_own_copy() {
        remember("one\r\ntwo");
        assert_eq!(own_copy().as_deref(), Some("one\ntwo"));
        assert!(own_copy_current());
        forget();
        assert!(!own_copy_current());
        assert_eq!(decide(own_copy().as_deref(), false, false), PasteRoute::Prompt);
    }

    #[test]
    fn prompt_waits_for_ctrl_v_or_cancel() {
        show_prompt();
        prompt_frame(false, false);
        assert!(prompt_shown());
        prompt_frame(true, false);
        assert!(!prompt_shown());
        show_prompt();
        prompt_frame(false, true);
        assert!(!prompt_shown());
    }

    #[test]
    fn dont_explain_again_is_remembered() {
        assert!(explain());
        set_explain(false);
        assert!(!explain());
        set_explain(true);
        assert!(explain());
    }
}
