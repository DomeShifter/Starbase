//! The browser's "Leave site?" prompt for unfinished work. A page cannot block Ctrl+W, but the
//! browser asks first when a `beforeunload` handler calls `preventDefault`. One handler is
//! installed the first time the guard is set; it prompts only while the guard is on.

/// Whether leaving now would lose something: a cell is being typed in, or the open file has
/// changes that are not saved yet. Everything saved: never prompt.
pub fn should_guard(editing_cell: bool, unsaved: bool) -> bool {
    editing_cell || unsaved
}

#[cfg(target_arch = "wasm32")]
pub use web::set;

#[cfg(target_arch = "wasm32")]
mod web {
    use std::cell::{Cell, RefCell};
    use wasm_bindgen::JsCast as _;

    thread_local! {
        static ON: Cell<bool> = const { Cell::new(false) };
        static INSTALLED: RefCell<bool> = const { RefCell::new(false) };
    }

    /// Turns the guard on or off (cheap: call it every frame).
    pub fn set(on: bool) {
        ON.with(|c| c.set(on));
        if !on || INSTALLED.with(|i| std::mem::replace(&mut *i.borrow_mut(), true)) {
            return;
        }
        let Some(window) = web_sys::window() else { return };
        let handler = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(|e: web_sys::BeforeUnloadEvent| {
            if ON.with(Cell::get) {
                e.prevent_default();
                // Older browsers ask only when `returnValue` is set.
                e.set_return_value("unsaved changes");
            }
        });
        if window.add_event_listener_with_callback("beforeunload", handler.as_ref().unchecked_ref()).is_err() {
            log::error!("no unsaved-changes guard");
        }
        // The listener lives as long as the page.
        handler.forget();
    }
}

#[cfg(test)]
mod tests {
    use super::should_guard;

    #[test]
    fn guards_only_while_something_is_unsaved() {
        // (editing a cell, file dirty) -> guard
        for (edit, dirty, want) in [(false, false, false), (true, false, true), (false, true, true), (true, true, true)] {
            assert_eq!(should_guard(edit, dirty), want, "edit={edit} dirty={dirty}");
        }
    }
}
