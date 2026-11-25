//! Who gets the keyboard each frame: one place instead of a fix per widget (review item 5).
//!
//! GridCraft's grid takes every key when no text box has the keyboard. Our dialogs are drawn
//! after the grid (they sit on top of it), so without this a dialog with only buttons and
//! drop-downs (Settings, the merge warning, "Open link?") let typing, Delete or Enter edit the
//! cells behind it. egui's Modal stops clicks reaching the grid, but not keys.
//!
//! The rules, in order, decided just before the grid draws:
//! 1. A one-line box let go of the keyboard this frame (Enter or Tab in a panel, menu or the
//!    toolbar): the key was the box's, so the grid doesn't see this frame's keys (to-do row 57).
//! 2. A house dialog was showing last frame: the keys are held back while the grid draws and
//!    handed on to the dialogs drawn after it. If no dialog was drawn after all (it closed just
//!    before the key came), the held keys are replayed to the grid on the next frame, so a key
//!    typed right after closing a dialog is never lost.
//! 3. Otherwise the grid gets the keys as usual.
//!
//! Every house dialog marks itself with [`note_modal`] (house::dialog does it), so a new dialog
//! is covered without being listed anywhere.

use egui::{Context, Event, Id, RawInput};

const MODAL_FRAME: &str = "starsheets_input_modal_frame";

/// Marks that a modal dialog is showing in this frame (called by house::dialog).
pub fn note_modal(ctx: &Context) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| d.insert_temp(Id::unique(MODAL_FRAME), frame));
}

/// Whether a house dialog showed in this frame or the last (cards hide behind no dialog).
pub fn modal_showing(ctx: &Context) -> bool {
    modal_frame(ctx).is_some_and(|f| f + 1 >= ctx.cumulative_frame_nr())
}

/// The last frame a modal dialog was shown in.
fn modal_frame(ctx: &Context) -> Option<u64> {
    ctx.data(|d| d.get_temp::<u64>(Id::unique(MODAL_FRAME)))
}

/// Keyboard events: the ones the grid would act on.
fn is_key(e: &Event) -> bool {
    matches!(e, Event::Key { .. } | Event::Text(_) | Event::Paste(_) | Event::Copy | Event::Cut | Event::Ime(_))
}

#[derive(Default)]
pub struct Router {
    /// Keys kept from the grid this frame (rule 2).
    held: Vec<Event>,
    /// Keys to give the grid next frame: held for a dialog that turned out to be closed.
    replay: Vec<Event>,
}

impl Router {
    /// Puts last frame's unused held keys back at the front of this frame's input.
    pub fn raw_input(&mut self, raw: &mut RawInput) {
        if !self.replay.is_empty() {
            let mut events = std::mem::take(&mut self.replay);
            events.append(&mut raw.events);
            raw.events = events;
        }
    }

    /// Just before the grid draws: rules 1 and 2. `focus_at_start` is which widget had the
    /// keyboard as the frame began.
    pub fn before_grid(&mut self, ctx: &Context, focus_at_start: Option<Id>) {
        self.held.clear();
        if focus_at_start.is_some() && ctx.memory(|m| m.focused()).is_none() {
            ctx.input_mut(|i| i.events.retain(|e| !is_key(e)));
            return;
        }
        let frame = ctx.cumulative_frame_nr();
        if modal_frame(ctx).is_some_and(|f| f + 1 >= frame) {
            ctx.input_mut(|i| {
                let (keys, rest): (Vec<Event>, Vec<Event>) = std::mem::take(&mut i.events).into_iter().partition(is_key);
                i.events = rest;
                self.held = keys;
            });
        }
    }

    /// Right after the grid: the held keys go back for the dialogs drawn next.
    pub fn after_grid(&mut self, ctx: &Context) {
        if !self.held.is_empty() {
            let held = self.held.clone();
            ctx.input_mut(|i| i.events.extend(held));
        }
    }

    /// At the end of the frame: held keys that no dialog was there to take go to the grid next
    /// frame.
    pub fn end_frame(&mut self, ctx: &Context) {
        if self.held.is_empty() {
            return;
        }
        let shown_now = modal_frame(ctx) == Some(ctx.cumulative_frame_nr());
        let held = std::mem::take(&mut self.held);
        if !shown_now {
            self.replay = held;
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> RawInput {
        RawInput { events: vec![Event::Text(text.into())], ..Default::default() }
    }

    /// One frame: the "grid" collects the text it sees, then a dialog (when `dialog`) draws.
    fn frame(ctx: &Context, router: &mut Router, mut raw: RawInput, dialog: bool) -> (String, String) {
        router.raw_input(&mut raw);
        let (mut grid, mut after) = (String::new(), String::new());
        let mut out = ctx.run_ui(raw, |ui| {
            let ctx = ui.ctx().clone();
            router.before_grid(&ctx, None);
            ctx.input(|i| {
                i.events.iter().for_each(|e| {
                    if let Event::Text(t) = e {
                        grid.push_str(t)
                    }
                })
            });
            router.after_grid(&ctx);
            if dialog {
                note_modal(&ctx);
                ctx.input(|i| {
                    i.events.iter().for_each(|e| {
                        if let Event::Text(t) = e {
                            after.push_str(t)
                        }
                    })
                });
            }
            router.end_frame(&ctx);
        });
        // The test has no painter: drop the frame's texture changes on purpose.
        out.textures_delta.clear();
        (grid, after)
    }

    #[test]
    fn keys_go_to_an_open_dialog_not_the_grid() {
        let ctx = Context::default();
        let mut r = Router::default();
        assert_eq!(frame(&ctx, &mut r, typed("a"), false).0, "a", "no dialog: the grid types");
        frame(&ctx, &mut r, RawInput::default(), true);
        assert_eq!(frame(&ctx, &mut r, typed("b"), true), (String::new(), "b".into()), "dialog open: it gets the key");
    }

    #[test]
    fn a_key_right_after_a_dialog_closes_reaches_the_grid() {
        let ctx = Context::default();
        let mut r = Router::default();
        frame(&ctx, &mut r, RawInput::default(), true);
        // The dialog closed without drawing again; the key waits one frame, then the grid has it.
        assert_eq!(frame(&ctx, &mut r, typed("c"), false).0, "");
        assert_eq!(frame(&ctx, &mut r, RawInput::default(), false).0, "c");
    }
}
