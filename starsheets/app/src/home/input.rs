//! The home screen's dragging, mouse back/forward buttons and keyboard shortcuts.

use super::*;

impl Home {
    /// Dragging: a label follows the pointer; letting go over a folder (card, tree row or path)
    /// moves there.
    pub(super) fn drag_ui(&mut self, ctx: &egui::Context) {
        let target = ctx.data_mut(|m| m.remove_temp::<String>(egui::Id::unique("home_drop")));
        let Some(items) = self.dragging.clone() else { return };
        if let Some(pos) = ctx.pointer_latest_pos() {
            let label = if items.len() == 1 { self.label_of(&items[0]) } else { format!("{} items", items.len()) };
            egui::Area::new(egui::Id::unique("home_drag")).fixed_pos(pos + vec2(14.0, 10.0)).order(egui::Order::Tooltip).interactable(false).show(
                ctx,
                |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(label);
                    });
                },
            );
        }
        if ctx.input(|i| i.pointer.any_released()) {
            self.dragging = None;
            if let Some(to) = target {
                self.send_to(ctx, items, to, true);
            }
        }
    }

    /// The mouse's back and forward buttons go Back and Forward through the places you visited
    /// (not Up: Up is the parent folder, Back is where you came from), instead of the browser
    /// leaving the page: index.html stops the browser's own handling while the home screen is up
    /// (`__ssHomeActive`) and counts the presses (`__ssHomeUp` = back, `__ssHomeFwd`);
    /// `__ssAtTop` (nothing to go back to) lets the browser's back button leave.
    #[cfg(target_arch = "wasm32")]
    pub(super) fn mouse_buttons(&mut self, _ctx: &egui::Context) {
        let Some(w) = web_sys::window() else { return };
        let get = |k: &str| js_sys::Reflect::get(&w, &k.into()).ok().and_then(|v| v.as_f64()).unwrap_or(0.0) as usize;
        let set = |k: &str, v: wasm_bindgen::JsValue| {
            let _ = js_sys::Reflect::set(&w, &k.into(), &v);
        };
        set("__ssHomeActive", true.into());
        let (up, fwd) = (get("__ssHomeUp"), get("__ssHomeFwd"));
        if up > 0 {
            set("__ssHomeUp", 0.into());
            for _ in 0..up {
                self.go_back();
            }
        }
        if fwd > 0 {
            set("__ssHomeFwd", 0.into());
            for _ in 0..fwd {
                self.go_forward();
            }
        }
        set("__ssAtTop", self.back.is_empty().into());
    }

    /// A browser that still went back (the history fallback in index.html) gives no input event,
    /// so the home screen looks once a second; a real button press wakes it at once.
    #[cfg(target_arch = "wasm32")]
    pub(super) fn keep_listening(ctx: &egui::Context) {
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
    }

    /// The desktop app gets the mouse's back and forward buttons as input events.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn mouse_buttons(&mut self, ctx: &egui::Context) {
        let (back, fwd) =
            ctx.input(|i| (i.pointer.button_pressed(egui::PointerButton::Extra1), i.pointer.button_pressed(egui::PointerButton::Extra2)));
        if back {
            self.go_back();
        }
        if fwd {
            self.go_forward();
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn keep_listening(_ctx: &egui::Context) {}

    pub(super) fn keys(&mut self, ctx: &egui::Context) {
        self.mouse_buttons(ctx);
        Self::keep_listening(ctx);
        if self.renaming.is_some()
            || ctx.memory(|m| m.focused()).is_some()
            || self.pending_move.is_some()
            || self.move_to.is_some()
            || self.details.is_some()
        {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            match ev {
                egui::Event::Copy if !self.selected.is_empty() => self.clipboard = Some((self.selected.clone(), false)),
                egui::Event::Cut if !self.selected.is_empty() => self.clipboard = Some((self.selected.clone(), true)),
                egui::Event::Paste(_) => {
                    if let Some(h) = self.here() {
                        self.paste_into(ctx, h);
                    }
                }
                egui::Event::Key { key, pressed: true, modifiers, .. } => match key {
                    Key::Enter if self.selected.len() == 1 => {
                        let item = self.selected[0].clone();
                        self.activate(ctx, &item);
                    }
                    Key::F2 if self.selected.len() == 1 => {
                        let item = self.selected[0].clone();
                        if super::may_edit(self.role_of(&item)) {
                            self.start_rename(&item);
                        }
                    }
                    Key::Delete if !self.selected.is_empty() => {
                        let items = std::mem::take(&mut self.selected);
                        self.delete(ctx, items);
                    }
                    Key::Backspace => self.go_up(),
                    Key::A if modifiers.command => self.selected = self.order(),
                    _ => {}
                },
                _ => {}
            }
        }
    }
}
