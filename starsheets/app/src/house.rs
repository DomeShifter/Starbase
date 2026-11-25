//! The StarSheets house style: one set of sizes, colours and shapes, taken from Google Sheets,
//! that every dialog, menu, card and panel uses (docs/house-style.md explains each choice).
//! Build new UI from these helpers rather than with numbers of its own.

use egui::{Align, Color32, CornerRadius, Frame, Layout, Margin, RichText, Sense, Stroke, Ui, vec2};

use crate::theme;

// ── Spacing: a 4 px scale ────────────────────────────────────────────────────────────────
pub const XS: f32 = 4.0;
pub const S: f32 = 8.0;
pub const M: f32 = 12.0;
pub const L: f32 = 16.0;
pub const XL: f32 = 24.0;

// ── Type ─────────────────────────────────────────────────────────────────────────────────
/// Dialog titles.
pub const TITLE: f32 = 22.0;
/// Field labels and section headings inside dialogs and panels.
pub const LABEL: f32 = 14.0;
/// Everything else people read: menu rows, values, buttons.
pub const BODY: f32 = 14.0;
/// Help text under a label.
pub const NOTE: f32 = 12.0;
/// Small capitals headings, such as STANDARD and CUSTOM in the colour menu.
pub const CAPS: f32 = 11.0;

// ── Colours (on top of theme.rs) ─────────────────────────────────────────────────────────
/// Sheets' green for the main button in a dialog, and the text of the others.
pub const GREEN: Color32 = Color32::from_rgb(0x14, 0x6C, 0x2E);
/// Google's blue: the Share dialog's buttons and Starbase Drive's accent (as in Starbase Docs).
pub const BLUE: Color32 = Color32::from_rgb(0x0B, 0x57, 0xD0);
/// The soft blue behind a selected item (a chosen list row, a picked chip).
pub const SELECTED: Color32 = Color32::from_rgb(0xE8, 0xF0, 0xFE);
/// The scroll bar handle (and egui's resting widget fill outside forms).
pub const SCROLL_HANDLE: Color32 = Color32::from_rgb(0xDA, 0xDC, 0xE0);
/// Error text.
pub const ERROR: Color32 = Color32::from_rgb(0xB3, 0x26, 0x1E);

// ── Shapes ───────────────────────────────────────────────────────────────────────────────
/// Dialogs, menus and cards.
pub const PANEL_RADIUS: u8 = 8;
/// Text boxes, drop-downs and chips.
pub const FIELD_RADIUS: u8 = 4;
/// Buttons are pills: half their 36 px height.
pub const BUTTON_RADIUS: u8 = 18;
pub const BUTTON_HEIGHT: f32 = 36.0;
pub const FIELD_HEIGHT: f32 = 36.0;
/// Menu rows and list rows.
pub const ROW_HEIGHT: f32 = 32.0;
/// Extra width a side panel keeps for its scroll bar (10 px bar and a gap), so content never
/// shifts or is cut when the panel starts to scroll.
pub const SCROLL_ROOM: f32 = 14.0;
/// How tall a side panel's scrolling part may be: down to the sheet-tab bar (40 px) and the
/// panel's own padding, so a long panel scrolls instead of being cut off.
pub fn panel_scroll_height(ui: &Ui) -> f32 {
    (ui.ctx().content_rect().bottom() - 40.0 - L - ui.cursor().top()).max(120.0)
}

/// The space between a menu's edge and anything in it that isn't a row (palettes, headings).
pub const MENU_INSET: i8 = 16;

// ---------------------------------------------------------------- menus

/// Every menu (menu bar, right-click, the New button, the avatar menu…) the Sheets way: flat
/// full-width rows, 16 px from the sides, 32 px tall, grey on hover, no corners on the rows.
/// Call first thing inside the menu's closure; `width` is the menu's width (220 plain, 320 with
/// shortcuts or icons).
pub fn menu(ui: &mut Ui, width: f32) {
    ui.set_min_width(width);
    ui.set_max_width(width);
    let st = ui.style_mut();
    st.spacing.button_padding = vec2(f32::from(MENU_INSET), 7.0);
    st.spacing.item_spacing = vec2(14.0, 0.0);
    st.spacing.icon_width = 10.0;
    let w = &mut st.visuals.widgets;
    for (state, fill) in [
        (&mut w.inactive, Color32::TRANSPARENT),
        (&mut w.hovered, theme::MENU_HOVER),
        (&mut w.active, theme::HOVER),
        (&mut w.open, theme::MENU_HOVER),
    ] {
        state.bg_stroke = Stroke::NONE;
        state.weak_bg_fill = fill;
        state.corner_radius = CornerRadius::ZERO;
        state.expansion = 0.0;
    }
}

/// HOUSE RULE (James, to-do 2 row 8): a list menu (sheets, drop-down choices, anything whose rows
/// are names) is only as wide as its longest row, plus the 16 px row padding each side, the icon
/// column when it has one, and a little room. Fixed widths (220 / 320) are for command menus.
pub fn fit_width<'a>(ui: &Ui, labels: impl IntoIterator<Item = &'a str>, icon_column: bool) -> f32 {
    let font = egui::FontId::proportional(BODY);
    let widest = labels.into_iter().map(|t| ui.painter().layout_no_wrap(t.to_string(), font.clone(), theme::TEXT).size().x).fold(0.0_f32, f32::max);
    let icon = if icon_column { 20.0 + 14.0 } else { 0.0 };
    (widest + 2.0 * f32::from(MENU_INSET) + icon + 12.0).clamp(80.0, 480.0).ceil()
}

/// A menu row: an icon (Material Symbols name, or none), the label, and a grey shortcut at the
/// right. Returns true when clicked.
pub fn menu_row(ui: &mut Ui, icon: Option<&str>, label: &str, keys: &str, enabled: bool) -> bool {
    let text = RichText::new(label).size(BODY).color(theme::TEXT);
    let b = match icon.and_then(crate::icons::icon) {
        Some(img) => egui::Button::image_and_text(img.fit_to_exact_size(vec2(20.0, 20.0)), text),
        None if icon.is_some() => egui::Button::new(RichText::new(format!("       {label}")).size(BODY).color(theme::TEXT)),
        None => egui::Button::new(text),
    };
    let b = if keys.is_empty() { b } else { b.shortcut_text(RichText::new(keys).size(NOTE).color(theme::TEXT_DIM)) };
    ui.add_enabled(enabled, b.min_size(vec2(ui.available_width(), 32.0))).clicked()
}

/// A menu divider: a light line with 8 px above and below.
pub fn menu_sep(ui: &mut Ui) {
    ui.add_space(S);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, theme::DIVIDER));
    ui.add_space(S);
}

// ---------------------------------------------------------------- text fields

/// A one-line text field the house way: the text centred top to bottom in the box (never riding
/// high), BODY size (SEARCH_TEXT for search bars), with room at the sides. Use it for every
/// single-line field: search bars, rename boxes, name fields.
pub fn field<'t>(text: &'t mut dyn egui::TextBuffer, size: f32) -> egui::TextEdit<'t> {
    egui::TextEdit::singleline(text).font(egui::FontId::proportional(size)).align(egui::Align2::LEFT_CENTER).margin(vec2(S, XS))
}

/// Text size in search bars.
pub const SEARCH_TEXT: f32 = 16.0;

// ---------------------------------------------------------------- folder colours

/// The colours a folder can have (right-click › Folder colour). Muted, distinct from each other,
/// readable on white and on the light card fill; no plain primaries. A folder without a colour
/// is drawn as an outline in the text grey.
pub const FOLDER_COLOURS: [(&str, Color32); 14] = [
    ("Slate", Color32::from_rgb(0x5B, 0x6B, 0x7F)),
    ("Ink", Color32::from_rgb(0x34, 0x49, 0x5E)),
    ("Ocean", Color32::from_rgb(0x2E, 0x7D, 0xA3)),
    ("Periwinkle", Color32::from_rgb(0x6C, 0x7F, 0xD1)),
    ("Lavender", Color32::from_rgb(0x8E, 0x7C, 0xC3)),
    ("Plum", Color32::from_rgb(0x8E, 0x4A, 0x87)),
    ("Rose", Color32::from_rgb(0xC2, 0x57, 0x7A)),
    ("Coral", Color32::from_rgb(0xE0, 0x7A, 0x5F)),
    ("Terracotta", Color32::from_rgb(0xB5, 0x52, 0x3B)),
    ("Sand", Color32::from_rgb(0xB0, 0x8D, 0x57)),
    ("Mustard", Color32::from_rgb(0xC9, 0xA2, 0x27)),
    ("Olive", Color32::from_rgb(0x8A, 0x9A, 0x3B)),
    ("Sage", Color32::from_rgb(0x6E, 0x9E, 0x75)),
    ("Teal", Color32::from_rgb(0x2A, 0x9D, 0x8F)),
];

/// "#RRGGBB" for a colour (how folder colours are stored).
pub fn hex(c: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())
}

/// A stored folder colour, if it is one of FOLDER_COLOURS.
pub fn folder_colour(hex_text: &str) -> Option<Color32> {
    FOLDER_COLOURS.iter().map(|(_, c)| *c).find(|c| hex(*c).eq_ignore_ascii_case(hex_text))
}

/// Applies the parts of the house style that egui can carry for everything at once, including
/// GridCraft's own windows: window padding, corners and shadows.
pub fn install(ctx: &egui::Context) {
    ctx.all_styles_mut(|s| {
        s.spacing.window_margin = Margin::same(XL as i8);
        s.spacing.menu_margin = Margin::symmetric(0, S as i8);
        // Solid scroll bars, always drawn when a list doesn't fit (egui's default thin ones only
        // show on hover, so a long list didn't look scrollable).
        // Solid bars, always drawn when a list doesn't fit (egui's default thin ones only show on
        // hover, so a long list didn't look scrollable). A solid handle takes the widget fill
        // colour, so forms keep their white fields inside the scrolling content, not around it.
        s.spacing.scroll = egui::style::ScrollStyle { bar_width: 10.0, handle_min_length: 24.0, ..egui::style::ScrollStyle::solid() };
        s.visuals.widgets.inactive.bg_fill = SCROLL_HANDLE;
        s.visuals.window_corner_radius = CornerRadius::same(PANEL_RADIUS);
        s.visuals.menu_corner_radius = CornerRadius::same(PANEL_RADIUS);
        s.visuals.window_stroke = Stroke::NONE;
        s.visuals.window_fill = Color32::WHITE;
        s.visuals.window_shadow = egui::epaint::Shadow { offset: [0, 4], blur: 24, spread: 0, color: Color32::from_black_alpha(60) };
        s.visuals.popup_shadow = egui::epaint::Shadow { offset: [0, 2], blur: 10, spread: 0, color: Color32::from_black_alpha(45) };
        // Menus, pop-ups and tooltips appear at once, as in Sheets: egui fades every new area in
        // over `animation_time` (0.2 s by default), which made menus feel slow (to-do row 80).
        // Also ends the expand/collapse glide on sections; scrolling keeps its own smoothing.
        s.animation_time = 0.0;
    });
}

/// Puts the house style back if something replaced it (GridCraft re-applies its own look when it
/// sets up fonts). Cheap: a check of two values most frames.
pub fn keep(ctx: &egui::Context) {
    let st = ctx.global_style();
    if st.spacing.scroll.bar_width != 10.0
        || st.visuals.window_corner_radius != CornerRadius::same(PANEL_RADIUS)
        || st.visuals.widgets.inactive.bg_fill != SCROLL_HANDLE
        || st.animation_time != 0.0
    {
        install(ctx);
    }
}

/// Tooltips: dark grey with white text (Google's look). egui draws them in the menu frame, so the
/// tooltip layer is restyled after the frame is painted: call once per frame, after all UI.
pub const TIP_BG: Color32 = Color32::from_rgb(0x3C, 0x40, 0x43);
/// The picked row in a dark popup (white text stays readable on it).
pub const TIP_PICKED: Color32 = Color32::from_rgb(0x0B, 0x57, 0xD0);
/// A hovered row in a dark popup.
pub const TIP_HOVER: Color32 = Color32::from_rgb(0x5F, 0x63, 0x68);

pub fn style_tooltips(ctx: &egui::Context) {
    let layers: Vec<egui::LayerId> = ctx.memory(|m| m.layer_ids().filter(|l| l.order == egui::Order::Tooltip).collect());
    if layers.is_empty() {
        return;
    }
    let fill = ctx.global_style().visuals.window_fill;
    ctx.graphics_mut(|g| {
        for l in layers {
            let Some(list) = g.get_mut(l) else { continue };
            for i in 0..list.next_idx().0 {
                list.mutate_shape(egui::layers::ShapeIdx(i), |cs| {
                    if restyle_tip(&mut cs.shape, fill) {
                        cs.clip_rect = cs.clip_rect.expand2(vec2(S, XS));
                    }
                });
            }
        }
    });
}

/// Restyles one tooltip shape; true when it grew (the background, given side padding).
fn restyle_tip(shape: &mut egui::Shape, fill: Color32) -> bool {
    match shape {
        egui::Shape::Rect(r) if r.fill == fill => {
            r.fill = TIP_BG;
            r.stroke = Stroke::NONE;
            r.corner_radius = CornerRadius::same(FIELD_RADIUS);
            r.rect = r.rect.expand2(vec2(S, 0.0));
            true
        }
        // Rows inside dark popups (GridCraft's formula suggestions share the tooltip layer): a
        // light highlight under white text can't be read, so a picked row turns strong blue and
        // a hovered one mid grey.
        egui::Shape::Rect(r) if r.fill.a() > 0 && u16::from(r.fill.r()) + u16::from(r.fill.g()) + u16::from(r.fill.b()) > 540 => {
            let bluish = r.fill.b() > r.fill.r().saturating_add(20);
            r.fill = if bluish { TIP_PICKED } else { TIP_HOVER };
            false
        }
        egui::Shape::Text(t) => {
            t.override_text_color = Some(Color32::WHITE);
            false
        }
        egui::Shape::Vec(v) => v.iter_mut().fold(false, |grew, s| restyle_tip(s, fill) | grew),
        _ => false,
    }
}

/// The frame of every dialog: white, 24 px padding, 8 px corners, a soft shadow.
pub fn dialog_frame() -> Frame {
    Frame::NONE
        .fill(Color32::WHITE)
        .inner_margin(Margin::same(XL as i8))
        .corner_radius(CornerRadius::same(PANEL_RADIUS))
        .shadow(egui::epaint::Shadow { offset: [0, 4], blur: 24, spread: 0, color: Color32::from_black_alpha(60) })
}

/// The frame of a floating card (a note, a link, the date picker): 16 px padding.
pub fn card_frame() -> Frame {
    Frame::NONE
        .fill(Color32::WHITE)
        .inner_margin(Margin::same(L as i8))
        .corner_radius(CornerRadius::same(PANEL_RADIUS))
        .shadow(egui::epaint::Shadow { offset: [0, 2], blur: 10, spread: 0, color: Color32::from_black_alpha(45) })
}

/// A small floating bubble (the calculation preview): 8 x 4 padding, 4 px corners.
pub fn bubble_frame() -> Frame {
    card_frame().inner_margin(Margin::symmetric(S as i8, XS as i8)).corner_radius(CornerRadius::same(FIELD_RADIUS))
}

/// The frame of a side panel (data validation, conditional formatting): 16 px all round.
pub fn panel_frame() -> Frame {
    Frame::NONE.fill(Color32::WHITE).inner_margin(Margin::same(L as i8)).stroke(Stroke::new(1.0, theme::DIVIDER))
}

/// Sets a dialog or panel `Ui` up in the house style: 14 px text, 36 px fields with a thin grey
/// outline and 4 px corners, 8 px between items.
pub fn form_style(ui: &mut Ui) {
    let st = ui.style_mut();
    st.spacing.item_spacing = vec2(S, S);
    st.spacing.button_padding = vec2(M, S);
    st.spacing.interact_size.y = FIELD_HEIGHT;
    st.spacing.combo_height = 320.0;
    for ts in [egui::TextStyle::Body, egui::TextStyle::Button] {
        st.text_styles.insert(ts, egui::FontId::proportional(BODY));
    }
    let w = &mut st.visuals.widgets;
    for (state, stroke) in [(&mut w.inactive, theme::BOX), (&mut w.hovered, theme::TEXT), (&mut w.active, theme::BLUE), (&mut w.open, theme::BLUE)] {
        state.bg_fill = Color32::WHITE;
        state.weak_bg_fill = Color32::WHITE;
        state.bg_stroke = Stroke::new(1.0, stroke);
        state.corner_radius = CornerRadius::same(FIELD_RADIUS);
        state.expansion = 0.0;
    }
    st.visuals.selection.stroke = Stroke::new(2.0, theme::BLUE);
}

/// Shows a dialog in the house style: title (with ✕ when `close` is given), then the body.
pub fn dialog<R>(ctx: &egui::Context, id: &str, width: f32, title: &str, close: Option<&mut bool>, body: impl FnOnce(&mut Ui) -> R) -> R {
    // While it shows, keys are the dialog's, not the grid's (input.rs).
    crate::input::note_modal(ctx);
    egui::Modal::new(egui::Id::unique(id))
        .frame(dialog_frame())
        .show(ctx, |ui| {
            form_style(ui);
            ui.set_width(width);
            // The ✕ is placed first so a long title is cut short ("…", full title on hover)
            // instead of running under it (to-do row 73).
            ui.horizontal(|ui| {
                let label = egui::Label::new(RichText::new(title).size(TITLE).color(theme::TEXT));
                match close {
                    Some(close) => {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if close_button(ui) {
                                *close = true;
                            }
                            ui.add_space(S);
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                ui.add(label.truncate()).on_hover_text(title);
                            });
                        });
                    }
                    None => {
                        ui.add(label.wrap());
                    }
                }
            });
            ui.add_space(L - S);
            body(ui)
        })
        .inner
}

/// A window's title row in the house style (title left, ✕ right), for windows drawn without
/// egui's own title bar. Returns true when ✕ was clicked.
#[cfg(target_arch = "wasm32")]
pub fn title_row(ui: &mut Ui, title: &str) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(TITLE).color(theme::TEXT));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| close = close_button(ui));
    });
    ui.add_space(L - S);
    close
}

/// A ✕ button for a dialog's corner.
pub fn close_button(ui: &mut Ui) -> bool {
    let b = match crate::icons::icon("close") {
        Some(img) => egui::Button::image(img.fit_to_exact_size(vec2(20.0, 20.0))),
        None => egui::Button::new(RichText::new("✕").size(16.0)),
    };
    ui.add(b.frame(false).min_size(vec2(32.0, 32.0))).on_hover_text("Close").clicked()
}

/// A field label, with its help text underneath when there is some.
pub fn label(ui: &mut Ui, text: &str, note: &str) {
    ui.label(RichText::new(text).size(LABEL).strong().color(theme::TEXT));
    if !note.is_empty() {
        self::note(ui, note);
    }
}

/// Help text: small and grey.
pub fn note(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(NOTE).color(theme::TEXT_DIM));
}

/// A note with more detail when the pointer is over it.
#[cfg(target_arch = "wasm32")]
pub fn note_with_tip(ui: &mut Ui, text: &str, tip: &str) {
    ui.label(RichText::new(text).size(NOTE).color(theme::TEXT_DIM)).on_hover_text(tip);
}

/// A small-capitals heading (STANDARD, CUSTOM).
pub fn caps(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text.to_uppercase()).size(CAPS).strong().color(theme::TEXT_DIM));
}

/// Body text.
pub fn text(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(BODY).color(theme::TEXT));
}

/// The main button: a green pill with white text.
pub fn primary(text: &str) -> egui::Button<'static> {
    primary_in(text, GREEN)
}

/// A main button in another accent ([`BLUE`] in the Share dialog, as in Google's).
pub fn primary_in(text: &str, accent: Color32) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).size(BODY).color(Color32::WHITE))
        .fill(accent)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(BUTTON_RADIUS))
        .min_size(vec2(88.0, BUTTON_HEIGHT))
}

/// Any other button: a white pill with a grey outline and green text.
pub fn secondary(text: &str) -> egui::Button<'static> {
    secondary_in(text, GREEN)
}

/// Any other button, its text in another accent.
pub fn secondary_in(text: &str, accent: Color32) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).size(BODY).color(accent))
        .fill(Color32::WHITE)
        .stroke(Stroke::new(1.0, theme::BOX))
        .corner_radius(CornerRadius::same(BUTTON_RADIUS))
        .min_size(vec2(88.0, BUTTON_HEIGHT))
}

/// A text-only button (green, no outline), for actions inside a list such as "Restore".
pub fn link(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).size(BODY).color(GREEN)).frame(false)
}

/// A dialog's buttons, at the bottom right with 24 px above and 8 px between. The last one is
/// the main (green) one. Returns the index of the one clicked.
pub fn actions(ui: &mut Ui, labels: &[&str]) -> Option<usize> {
    actions_in(ui, labels, GREEN)
}

/// A dialog's buttons in another accent.
pub fn actions_in(ui: &mut Ui, labels: &[&str], accent: Color32) -> Option<usize> {
    ui.add_space(XL - S);
    let mut hit = None;
    ui.allocate_ui_with_layout(vec2(ui.available_width(), BUTTON_HEIGHT), Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = S;
        for (i, l) in labels.iter().enumerate().rev() {
            let b = if i + 1 == labels.len() { primary_in(l, accent) } else { secondary_in(l, accent) };
            if ui.add(b).clicked() {
                hit = Some(i);
            }
        }
    });
    hit
}

/// Space between two groups of fields in a dialog or panel.
pub fn gap(ui: &mut Ui) {
    ui.add_space(L);
}

/// Content inside a menu that isn't a row (a palette, a heading), kept 16 px from the sides.
pub fn menu_inset<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::NONE.inner_margin(Margin::symmetric(MENU_INSET, XS as i8)).show(ui, add).inner
}

/// A drop-down field: a 36 px box with the chosen label and ▾, opening a Sheets menu with every
/// choice at once (a ✓ by the current one; `dividers_after` puts a line after those indexes).
/// Returns true when the choice changed.
pub fn select(ui: &mut Ui, id: &str, width: f32, current: &mut usize, labels: &[&str], dividers_after: &[usize]) -> bool {
    let shown = labels.get(*current).copied().unwrap_or_default();
    let field = egui::Button::new(RichText::new(shown).size(BODY).color(theme::TEXT))
        .right_text(RichText::new("▾").size(BODY).color(theme::TEXT_DIM))
        .fill(Color32::WHITE)
        .stroke(Stroke::new(1.0, theme::BOX))
        .corner_radius(CornerRadius::same(FIELD_RADIUS))
        .min_size(vec2(width, FIELD_HEIGHT));
    let r = ui.add(field);
    let mut changed = false;
    egui::Popup::menu(&r).id(egui::Id::unique(id)).show(|ui| {
        crate::shell::menu_style_width(ui, width.max(200.0));
        ui.spacing_mut().button_padding.x = 40.0;
        // As tall as the screen allows; a scroll bar when it still doesn't fit.
        egui::ScrollArea::vertical().max_height(menu_max_height(ui.ctx())).show(ui, |ui| {
            for (i, label) in labels.iter().enumerate() {
                let row = ui.add(
                    egui::Button::new(RichText::new(*label).size(BODY).color(theme::TEXT))
                        .shortcut_text("")
                        .frame_when_inactive(false)
                        .min_size(vec2(0.0, ROW_HEIGHT)),
                );
                if i == *current {
                    ui.painter().text(
                        row.rect.left_center() + vec2(L, 0.0),
                        egui::Align2::LEFT_CENTER,
                        "✓",
                        egui::FontId::proportional(16.0),
                        theme::TEXT,
                    );
                }
                if row.clicked() {
                    changed = i != *current;
                    *current = i;
                    ui.close();
                }
                if dividers_after.contains(&i) {
                    crate::shell::menu_sep_width(ui, width.max(200.0));
                }
            }
        });
    });
    changed
}

/// A row of tabs (Google's dialog tabs): text labels, the chosen one blue with a 3 px blue bar
/// under it, a 1 px divider under the whole row. Returns true when the choice changed.
pub fn tabs(ui: &mut Ui, current: &mut usize, labels: &[&str]) -> bool {
    let mut changed = false;
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (i, label) in labels.iter().enumerate() {
            let on = i == *current;
            let colour = if on { theme::BLUE } else { theme::TEXT_DIM };
            let text = RichText::new(*label).size(BODY).color(colour);
            let r =
                ui.add(egui::Button::new(if on { text.strong() } else { text }).frame_when_inactive(false).min_size(vec2(88.0, FIELD_HEIGHT + XS)));
            if on {
                let bar =
                    egui::Rect::from_min_max(egui::pos2(r.rect.left() + S, r.rect.bottom() - 3.0), egui::pos2(r.rect.right() - S, r.rect.bottom()));
                ui.painter().rect_filled(bar, CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 }, theme::BLUE);
            }
            if r.clicked() && !on {
                *current = i;
                changed = true;
            }
        }
    });
    let r = row.response.rect;
    ui.painter().hline(ui.max_rect().x_range(), r.bottom(), Stroke::new(1.0, theme::DIVIDER));
    ui.add_space(L);
    changed
}

/// The most a menu's list may take before it scrolls: the window's height less a margin.
pub fn menu_max_height(ctx: &egui::Context) -> f32 {
    (ctx.content_rect().height() - 2.0 * XL).max(160.0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_spacing_scale_steps_in_fours() {
        for v in [super::XS, super::S, super::M, super::L, super::XL] {
            assert_eq!(v % 4.0, 0.0);
        }
        assert_eq!(super::BUTTON_RADIUS as f32 * 2.0, super::BUTTON_HEIGHT);
    }
}
