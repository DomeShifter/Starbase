//! The StarSheets palette: GridCraft's tokens with Sheets-style colours (docs/sheets-reference.md §1).
//! Only colours change here; sizes and fonts follow later in phase 1.

use egui::Color32;
use gridcraft_ui_egui::theme::{self, Tokens};

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// Sheets blue (selection, accent).
pub const BLUE: Color32 = rgb(0x1A73E8);
/// Page background behind the chrome.
pub const PAGE: Color32 = rgb(0xF9FBFD);
/// The toolbar pill.
pub const TOOLBAR: Color32 = rgb(0xEDF2FA);

thread_local! {
    /// The toolbar background chosen in Settings › Style (None = TOOLBAR).
    static TOOLBAR_CHOICE: std::cell::Cell<Option<Color32>> = const { std::cell::Cell::new(None) };
}

/// The toolbar's background: the Settings › Style choice, or Google's pale blue.
pub fn toolbar() -> Color32 {
    TOOLBAR_CHOICE.with(|c| c.get()).unwrap_or(TOOLBAR)
}

/// Sets (or with None, resets) the toolbar background; the next frame draws it.
pub fn set_toolbar(colour: Option<Color32>) {
    TOOLBAR_CHOICE.with(|c| c.set(colour));
}
/// The Share button.
pub const SHARE: Color32 = rgb(0xC2E7FF);
pub const TEXT: Color32 = rgb(0x1F1F1F);
pub const TEXT_DIM: Color32 = rgb(0x444746);
pub const HOVER: Color32 = rgb(0xDDE3EA);
/// Menu row hover.
pub const MENU_HOVER: Color32 = rgb(0xF1F3F4);
/// Menu dividers.
pub const DIVIDER: Color32 = rgb(0xDADCE0);
/// Pressed buttons.
pub const PRESSED: Color32 = rgb(0xC9D3DF);
/// The thin box round the font size.
pub const BOX: Color32 = rgb(0x747775);
/// The active sheet tab's background.
pub const ACTIVE_TAB: Color32 = rgb(0xE1E9F7);

pub fn light() -> Tokens {
    let mut t = Tokens::light();
    t.window = PAGE;
    t.ribbon = PAGE;
    t.ribbon_border = rgb(0xE1E3E1);
    t.text = TEXT;
    t.text_dim = TEXT_DIM;
    t.accent = BLUE;
    t.accent_dark = rgb(0x0B57D0);
    t.accent_soft = rgb(0xD3E3FD);
    t.hover = HOVER;
    t.pressed = PRESSED;
    t.gridline = rgb(0xE2E3E3);
    t.header_bg = rgb(0xF8F9FA);
    t.header_text = TEXT_DIM;
    t.header_line = rgb(0xC4C7C5);
    t.header_sel_bg = rgb(0xD3E3FD);
    t.header_sel_text = rgb(0x0B57D0);
    t.header_all_bg = rgb(0xD3E3FD);
    t.sel_fill = Color32::from_rgba_unmultiplied(0x1A, 0x73, 0xE8, 0x1A);
    t.active_row = Color32::from_rgba_unmultiplied(0x1A, 0x73, 0xE8, 0x26);
    t.sel_border = BLUE;
    t.tab_bar = PAGE;
    t.tab_active = rgb(0xE1E9F7);
    t.status_bar = PAGE;
    t.sheet_fill = BLUE;
    t.sheet_focus = BLUE;
    t.error_check_green = rgb(0x188038);
    t.sheets_marks = true;
    t.sheets_formula_bar = true;
    t.on_accent = Color32::WHITE;
    t
}

/// Installs the palette (dark keeps GridCraft's for now) and re-applies egui's visuals.
pub fn install(ctx: &egui::Context) {
    Tokens::install(ctx, light(), Tokens::dark());
    theme::apply(ctx, false);
    // Padding, corners and shadows for every window, menu and dialog (house.rs).
    crate::house::install(ctx);
}
