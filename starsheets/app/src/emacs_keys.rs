//! One definition, shared by Sheets, Docs and Slides (the latter two include this file by path).
//!
//! egui's text box reads some Ctrl+letter chords the Emacs way: Ctrl+H is Backspace, Ctrl+K deletes
//! to the end of the line, Ctrl+U to the start, Ctrl+W the word before. No modern office app does
//! that (Ctrl+H is Find and replace, Ctrl+K a link, Ctrl+U underline, Ctrl+W close), so each app
//! calls [`strip`] first thing in its `raw_input_hook`: while a text box has focus these chords are
//! taken out of the input before egui sees them, and the app may ask [`take`] whether one was
//! pressed. An app with no feature for a chord simply ignores it: the chord does nothing.
//! Chords with Shift or Alt are left alone; so is everything when no text box has focus (the
//! document canvases read their own shortcuts).
#![allow(dead_code)]

use egui::{Event, Key};

/// The chords that egui's text box would read as Emacs editing.
pub const KEYS: [Key; 4] = [Key::H, Key::K, Key::U, Key::W];

#[derive(Clone, Default)]
struct Taken(Vec<Key>);

/// True for a Ctrl (or Cmd) press of H/K/U/W without Shift or Alt.
fn is_emacs_chord(e: &Event) -> Option<(Key, bool)> {
    match e {
        Event::Key { key, pressed, repeat, modifiers, .. }
            if KEYS.contains(key) && (modifiers.ctrl || modifiers.command || modifiers.mac_cmd) && !modifiers.shift && !modifiers.alt =>
        {
            Some((*key, *pressed && !*repeat))
        }
        _ => None,
    }
}

/// Removes the chords from `events`; returns the keys that were pressed.
pub fn filter(events: &mut Vec<Event>) -> Vec<Key> {
    let mut taken = Vec::new();
    events.retain(|e| match is_emacs_chord(e) {
        Some((key, fresh)) => {
            if fresh {
                taken.push(key);
            }
            false
        }
        None => true,
    });
    taken
}

/// Call from the host's `raw_input_hook`, before anything else reads the events.
pub fn strip(ctx: &egui::Context, raw: &mut egui::RawInput) {
    if !ctx.text_edit_focused() {
        return;
    }
    let taken = filter(&mut raw.events);
    if !taken.is_empty() {
        ctx.data_mut(|d| d.insert_temp(egui::Id::NULL.with("emacs_keys"), Taken(taken)));
    }
}

/// Whether `key` (with Ctrl/Cmd) was pressed in a text box and taken out by [`strip`]; asking
/// answers once.
pub fn take(ctx: &egui::Context, key: Key) -> bool {
    let id = egui::Id::NULL.with("emacs_keys");
    ctx.data_mut(|d| {
        let mut t: Taken = d.get_temp(id).unwrap_or_default();
        let hit = t.0.contains(&key);
        t.0.retain(|k| *k != key);
        d.insert_temp(id, t);
        hit
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Modifiers;

    fn press(key: Key, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }
    }

    #[test]
    fn filter_takes_ctrl_and_cmd_hkuw_only() {
        for key in KEYS {
            for m in [Modifiers::CTRL, Modifiers::COMMAND, Modifiers::MAC_CMD] {
                let mut ev = vec![press(key, m)];
                assert_eq!(filter(&mut ev), vec![key]);
                assert!(ev.is_empty());
            }
            for m in [Modifiers::NONE, Modifiers::CTRL | Modifiers::SHIFT, Modifiers::CTRL | Modifiers::ALT, Modifiers::SHIFT] {
                let mut ev = vec![press(key, m)];
                assert!(filter(&mut ev).is_empty());
                assert_eq!(ev.len(), 1, "{key:?} with {m:?} is left alone");
            }
        }
        // Other chords (copy, bold, find, backspace) pass through.
        let mut ev: Vec<Event> = [Key::C, Key::B, Key::F, Key::Backspace, Key::A].iter().map(|k| press(*k, Modifiers::CTRL)).collect();
        assert!(filter(&mut ev).is_empty());
        assert_eq!(ev.len(), 5);
    }

    use egui_kittest::Harness;

    /// A harness with one focused text box ("alpha beta gamma", cursor after "alpha b"), and a step
    /// that runs the shared filter on the frame's input first, as the host's `raw_input_hook` does.
    fn harness() -> Harness<'static, String> {
        let mut h = Harness::builder().with_accessibility_check(false).build_ui_state(
            |ui, text: &mut String| {
                let out = egui::TextEdit::singleline(text).id(egui::Id::NULL.with("box")).show(ui);
                out.response.request_focus();
            },
            String::from("alpha beta gamma"),
        );
        h.step();
        h.step();
        let mut st = egui::TextEdit::load_state(&h.ctx, egui::Id::NULL.with("box")).unwrap_or_default();
        st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(7))));
        st.store(&h.ctx, egui::Id::NULL.with("box"));
        h.step();
        h
    }

    fn step(h: &mut Harness<'static, String>, filtered: bool, key: Key) {
        h.input_mut().events.push(press(key, Modifiers::CTRL));
        if filtered {
            let mut raw = std::mem::take(h.input_mut());
            strip(&h.ctx, &mut raw);
            *h.input_mut() = raw;
        }
        h.step();
    }

    /// A focused text box never deletes text on Ctrl+H/K/U/W (cursor mid-text), and the app is told.
    #[test]
    fn focused_text_box_keeps_its_text() {
        for key in KEYS {
            let mut h = harness();
            assert!(h.ctx.text_edit_focused(), "box has focus");
            step(&mut h, true, key);
            assert_eq!(h.state(), "alpha beta gamma", "Ctrl+{key:?} must not edit text");
            assert!(take(&h.ctx, key), "the app is told");
            assert!(!take(&h.ctx, key), "and only once");
        }
    }

    /// Control: without the filter egui itself edits the text for these chords (what this prevents).
    #[test]
    fn egui_alone_would_have_edited() {
        let mut h = harness();
        step(&mut h, false, Key::H);
        assert_ne!(h.state(), "alpha beta gamma");
    }
}
