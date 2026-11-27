//! Starbase Slides' colours, light and dark. The bars we draw (title row, menus, toolbar) use the
//! Starbase house colours (the same light colours as Starbase Docs and Sheets; a Google-style
//! dark set); DeckCraft draws the slide list, the canvas and its dialogs with its own light or
//! dark tokens, switched with its `view.dark` command so both halves always match.
//!
//! Web Slides takes its appearance from the browser preference in Starbase settings.
//! Native Slides uses the light appearance.

use deckcraft_ui_egui::SlideApp;
use egui::Color32;
use serde_json::json;

/// The appearance of the editor shell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Light,
    Dark,
}

impl Mode {
    pub fn parse(s: &str) -> Mode {
        match s {
            "dark" => Mode::Dark,
            _ => Mode::Light,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Light => "light",
            Mode::Dark => "dark",
        }
    }

    /// Whether this means dark (never the system's preference).
    pub fn is_dark(self) -> bool {
        self == Mode::Dark
    }
}

/// The colours of our bars.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    /// The title row and menu bar.
    pub head: Color32,
    /// The toolbar's rounded strip.
    pub toolbar: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    /// Icons.
    pub icon: Color32,
    pub divider: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    /// A toolbar button that is on (Bold in bold text).
    pub on: Color32,
    pub menu: Color32,
    pub menu_hover: Color32,
    pub menu_line: Color32,
    pub keys: Color32,
    /// The Slideshow button.
    pub present: Color32,
    pub present_text: Color32,
    pub error: Color32,
    pub notice: Color32,
    /// Links and the main button in a dialog (Connect), and the text on that button.
    pub link: Color32,
    pub on_link: Color32,
}

impl Palette {
    pub fn light() -> Self {
        Palette {
            dark: false,
            head: Color32::from_rgb(0xF9, 0xFB, 0xFD),
            toolbar: Color32::from_rgb(0xED, 0xF2, 0xFA),
            text: Color32::from_rgb(0x1F, 0x1F, 0x1F),
            text_dim: Color32::from_rgb(0x44, 0x47, 0x46),
            icon: Color32::from_rgb(0x44, 0x47, 0x46),
            divider: Color32::from_rgb(0xC7, 0xC7, 0xC7),
            hover: Color32::from_rgb(0xDD, 0xE3, 0xEA),
            pressed: Color32::from_rgb(0xC9, 0xD3, 0xDF),
            on: Color32::from_rgb(0xD3, 0xE3, 0xFD),
            menu: Color32::WHITE,
            menu_hover: Color32::from_rgb(0xF1, 0xF3, 0xF4),
            menu_line: Color32::from_rgb(0xDA, 0xDC, 0xE0),
            keys: Color32::from_rgb(0x5F, 0x63, 0x68),
            present: Color32::from_rgb(0xFE, 0xEF, 0xC3),
            present_text: Color32::from_rgb(0x1F, 0x1F, 0x1F),
            error: Color32::from_rgb(0xB3, 0x26, 0x1E),
            notice: Color32::from_rgb(0x8A, 0x5A, 0x00),
            link: Color32::from_rgb(0x0B, 0x57, 0xD0),
            on_link: Color32::WHITE,
        }
    }

    pub fn dark() -> Self {
        Palette {
            dark: true,
            head: Color32::from_rgb(0x1B, 0x1B, 0x1B),
            toolbar: Color32::from_rgb(0x2D, 0x2F, 0x31),
            text: Color32::from_rgb(0xE3, 0xE3, 0xE3),
            text_dim: Color32::from_rgb(0xC4, 0xC7, 0xC5),
            icon: Color32::from_rgb(0xC4, 0xC7, 0xC5),
            divider: Color32::from_rgb(0x5F, 0x63, 0x68),
            hover: Color32::from_rgb(0x3C, 0x40, 0x43),
            pressed: Color32::from_rgb(0x4A, 0x4E, 0x51),
            on: Color32::from_rgb(0x00, 0x4A, 0x77),
            menu: Color32::from_rgb(0x2D, 0x2F, 0x31),
            menu_hover: Color32::from_rgb(0x3C, 0x40, 0x43),
            menu_line: Color32::from_rgb(0x44, 0x47, 0x4A),
            keys: Color32::from_rgb(0x9A, 0xA0, 0xA6),
            present: Color32::from_rgb(0x5C, 0x45, 0x00),
            present_text: Color32::from_rgb(0xFE, 0xEF, 0xC3),
            error: Color32::from_rgb(0xF2, 0xB8, 0xB5),
            notice: Color32::from_rgb(0xF6, 0xC3, 0x5B),
            link: Color32::from_rgb(0xA8, 0xC7, 0xFA),
            on_link: Color32::from_rgb(0x06, 0x2E, 0x6F),
        }
    }

    pub fn of(dark: bool) -> Self {
        if dark { Self::dark() } else { Self::light() }
    }
}

/// Makes DeckCraft's own tokens match `dark` (its `view.dark` command restyles on the next frame).
/// Only runs the command when they differ, so it costs nothing on most frames.
pub fn sync(app: &mut SlideApp, dark: bool) {
    if app.ui.brightness != brightness(dark) {
        let _ = app.run("view.dark", json!({ "on": dark }));
    }
}

fn brightness(dark: bool) -> deckcraft_ui_egui::theme::Brightness {
    if dark { deckcraft_ui_egui::theme::Brightness::Dark } else { deckcraft_ui_egui::theme::Brightness::Light }
}

/// The theme as the test hook (browser only) reports it.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn state(app: &SlideApp, mode: Mode, ctx: &egui::Context) -> serde_json::Value {
    json!({
        "mode": mode.name(),
        "dark": mode.is_dark(),
        "deckcraftDark": app.ui.brightness == brightness(true),
        "eguiDark": ctx.global_style().visuals.dark_mode,
    })
}
