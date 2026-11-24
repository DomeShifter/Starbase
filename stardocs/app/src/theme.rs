//! StarDocs is light whatever the system prefers, like Google Docs (light grey round a white
//! page). WordCraft's interface theme defaults to "Use system setting", so a browser that prefers
//! a dark colour scheme turned the page surround, rulers, hover fills, tooltips and the title box
//! dark under our light Docs header. Until StarDocs has a dark mode designed as a whole, the
//! interface theme is pinned to Light: at startup (before the first frame, so nothing flashes
//! dark) and on every frame (a stored or chosen "dark" doesn't come back).

use wordcraft_ui_egui::WordApp;
use wordcraft_ui_egui::theme::{Appearance, Tokens};

/// Pins WordCraft's interface theme to Light. WordCraft applies it (egui `ThemePreference::Light`)
/// in its next `logic` pass, so every egui surface takes the light visuals.
pub fn keep_light(app: &mut WordApp) {
    app.ui.theme = Appearance::Light;
}

/// What the test hook reports (`state().theme`): the system's preference, the theme egui draws
/// with, and the colours of the page surround, the rulers and a hovered button, as `#rrggbb`.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn state(ctx: &egui::Context) -> serde_json::Value {
    let name = |t: egui::Theme| if t == egui::Theme::Dark { "dark" } else { "light" };
    let hex = |c: egui::Color32| format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b());
    let tokens = Tokens::get(ctx);
    let visuals = ctx.global_style().visuals.clone();
    serde_json::json!({
        "system": ctx.system_theme().map(name),
        "interface": name(ctx.theme()),
        "surround": hex(tokens.canvas),
        "ruler": hex(tokens.ruler),
        "hover": hex(visuals.widgets.hovered.weak_bg_fill),
        "textBox": hex(visuals.text_edit_bg_color()),
    })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "tests")]

    use egui::{Color32, Theme};
    use egui_kittest::Harness;
    use serde_json::json;
    use wordcraft_engine::Session;
    use wordcraft_ui_egui::theme::Tokens;
    use wordcraft_ui_egui::{HostChrome, Services, WordApp};

    use crate::shell::Shell;

    struct Win {
        app: WordApp,
        shell: Shell,
    }

    /// Our window as web.rs draws it, on a system that prefers dark.
    fn dark_system_window(app: WordApp) -> Harness<'static, Win> {
        let mut h = Harness::builder().with_size(egui::vec2(1300.0, 800.0)).build_ui_state(
            |ui, w: &mut Win| {
                w.app.logic(ui.ctx());
                w.shell.ui(&mut w.app, ui, None);
                w.app.ui(ui);
            },
            Win { app, shell: Shell::default() },
        );
        for _ in 0..4 {
            h.input_mut().system_theme = Some(Theme::Dark);
            h.step();
        }
        h
    }

    fn new_app() -> WordApp {
        let mut doc = wordcraft_engine::blank_document(&Default::default());
        crate::shell::docs_defaults(&mut doc);
        let mut app =
            WordApp::new(Session::new(doc), Services::default()).with_chrome(HostChrome { title_bar: false, ribbon: false, status_bar: false });
        app.ui.language = "en".into();
        app.autosave = false;
        app
    }

    /// The visuals egui draws with are the light ones: page surround, hover and pressed fills,
    /// text boxes (the title), menus, tooltips and scrollbars all come from these.
    fn assert_light(h: &Harness<'static, Win>) {
        assert_eq!(h.ctx.system_theme(), Some(Theme::Dark), "the test really has a dark system");
        assert_eq!(h.ctx.theme(), Theme::Light, "the interface is light on a dark system");
        let v = h.ctx.global_style().visuals.clone();
        assert!(!v.dark_mode, "light visuals");
        let (t, light) = (Tokens::get(&h.ctx), Tokens::light());
        assert_eq!((t.canvas, t.ruler, t.panel, t.menu), (light.canvas, light.ruler, light.panel, light.menu), "WordCraft's light palette");
        let st = super::state(&h.ctx);
        assert_eq!((st["system"].as_str(), st["interface"].as_str()), (Some("dark"), Some("light")), "{st}");
        let lum = |c: Color32| (u32::from(c.r()) + u32::from(c.g()) + u32::from(c.b())) / 3;
        for (what, c) in [
            ("window/panel background", v.panel_fill),
            ("popups and menus", v.window_fill),
            ("text box (title) background", v.text_edit_bg_color()),
            ("hovered button", v.widgets.hovered.weak_bg_fill),
            ("pressed button", v.widgets.active.weak_bg_fill),
            ("faint background (page surround, scrollbars)", v.faint_bg_color),
            ("extreme background", v.extreme_bg_color),
        ] {
            assert!(lum(c) > 0x99, "{what} is light, got {c:?}");
        }
        assert!(lum(v.text_color()) < 0x80, "text is dark on the light interface, got {:?}", v.text_color());
        assert!(lum(v.text_cursor.stroke.color) < 0x80, "the text cursor shows on a light box");
    }

    /// The owner's report: on an OS/browser that prefers dark, the page surround and the egui
    /// hover/title box visuals went dark. They stay light.
    #[test]
    fn a_dark_system_keeps_the_light_interface() {
        let h = dark_system_window(new_app());
        assert_light(&h);
    }

    /// A "dark" theme stored by an earlier visit (or chosen in WordCraft's Options) doesn't come
    /// back either.
    #[test]
    fn a_stored_dark_theme_stays_light() {
        let mut app = new_app();
        app.run("ui.theme", json!({"value": "dark"})).unwrap();
        let mut h = dark_system_window(app);
        assert_light(&h);
        h.state_mut().app.run("ui.theme", json!({"value": "dark"})).unwrap();
        for _ in 0..3 {
            h.input_mut().system_theme = Some(Theme::Dark);
            h.step();
        }
        assert_light(&h);
    }
}
