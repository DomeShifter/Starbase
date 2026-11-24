//! The text and fill colour menus, as in Google Sheets: Reset, the palette (with a tick on the
//! colour the cell has), the standard colours, your custom colours with ⊕ for a new one (a picker
//! with a saturation square, a hue slider and Hex / R / G / B boxes), and a link to conditional
//! formatting. Custom colours are remembered in the browser.

use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Ui, vec2};
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

use crate::{house, theme};

/// Google's palette: a grey row, a bright row, then lighter to darker tints of the bright row.
const PALETTE: [[u32; 10]; 8] = [
    [0x000000, 0x434343, 0x666666, 0x999999, 0xB7B7B7, 0xCCCCCC, 0xD9D9D9, 0xEFEFEF, 0xF3F3F3, 0xFFFFFF],
    [0x980000, 0xFF0000, 0xFF9900, 0xFFFF00, 0x00FF00, 0x00FFFF, 0x4A86E8, 0x0000FF, 0x9900FF, 0xFF00FF],
    [0xE6B8AF, 0xF4CCCC, 0xFCE5CD, 0xFFF2CC, 0xD9EAD3, 0xD0E0E3, 0xC9DAF8, 0xCFE2F3, 0xD9D2E9, 0xEAD1DC],
    [0xDD7E6B, 0xEA9999, 0xF9CB9C, 0xFFE599, 0xB6D7A8, 0xA2C4C9, 0xA4C2F4, 0x9FC5E8, 0xB4A7D6, 0xD5A6BD],
    [0xCC4125, 0xE06666, 0xF6B26B, 0xFFD966, 0x93C47D, 0x76A5AF, 0x6D9EEB, 0x6FA8DC, 0x8E7CC3, 0xC27BA0],
    [0xA61C00, 0xCC0000, 0xE69138, 0xF1C232, 0x6AA84F, 0x45818E, 0x3C78D8, 0x3D85C6, 0x674EA7, 0xA64D79],
    [0x85200C, 0x990000, 0xB45F06, 0xBF9000, 0x38761D, 0x134F5C, 0x1155CC, 0x0B5394, 0x351C75, 0x741B47],
    [0x5B0F00, 0x660000, 0x783F04, 0x7F6000, 0x274E13, 0x0C343D, 0x1C4587, 0x073763, 0x20124D, 0x4C1130],
];

/// Google's "Standard" colours.
const STANDARD: [u32; 8] = [0x000000, 0xFFFFFF, 0x4285F4, 0xEA4335, 0xFBBC04, 0x34A853, 0xFF6D01, 0x46BDC6];

const CUSTOM_KEY: &str = "starsheets_custom_colours";
const CUSTOM_MAX: usize = 10;
/// Asks for the custom picker (the menu closes, the picker stays): the command it sets.
const PICKER_REQUEST: &str = "starsheets_custom_colour_for";

fn c32(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// The colour the active cell has now, for the tick: its text colour or fill.
fn current(app: &SheetApp, cmd: &str) -> Option<u32> {
    let d = app.session.active()?;
    let sh = d.wb.active()?;
    let st = d.wb.styles.get(sh.style_id(d.selection.active));
    let c = if cmd == "home.fontColor" { &st.font.color } else { &st.fill.fg };
    crate::cfpanel::rgb_of(c, &d.wb.theme)
}

/// What was chosen in a colour menu.
pub enum Choice {
    Reset,
    Colour(u32),
    /// ⊕: open the custom colour picker.
    Custom,
    /// The "Conditional formatting" link.
    Conditional,
}

/// The colour menu's body in Google's layout, for any colour choice: Reset, the palette (✓ on
/// `now`), STANDARD, CUSTOM with ⊕ (when `custom`), and the conditional formatting link (when
/// `conditional`). Closes the menu when something is chosen.
pub fn palette(ui: &mut Ui, now: Option<u32>, custom: bool, conditional: bool) -> Option<Choice> {
    let choice = palette_open(ui, now, custom, conditional);
    if choice.is_some() {
        ui.close();
    }
    choice
}

/// [`palette`] that leaves the menu open after a choice (the borders menu, to-do 2 row 26).
pub fn palette_open(ui: &mut Ui, now: Option<u32>, custom: bool, conditional: bool) -> Option<Choice> {
    // Rows run edge to edge like every menu; the palette sits 16 px in from the sides.
    let width = 10.0 * 19.0 + 9.0 * 4.0 + 2.0 * f32::from(house::MENU_INSET);
    ui.set_width(width);
    crate::shell::menu_style_width(ui, width);
    let text = |t: &str| RichText::new(t.to_string()).size(house::BODY).color(theme::TEXT);
    let mut choice = None;
    let reset = match crate::icons::icon("format_color_reset") {
        Some(img) => egui::Button::image_and_text(img.fit_to_exact_size(vec2(18.0, 18.0)), text("Reset")),
        None => egui::Button::new(text("Reset")),
    };
    if ui.add(reset.frame_when_inactive(false).min_size(vec2(0.0, house::ROW_HEIGHT))).clicked() {
        choice = Some(Choice::Reset);
    }
    let mut picked = None;
    house::menu_inset(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for row in PALETTE {
            swatches(ui, &row, now, &mut picked);
        }
        ui.add_space(house::S);
        house::caps(ui, "Standard");
        swatches(ui, &STANDARD, now, &mut picked);
    });
    if custom {
        crate::shell::menu_sep_width(ui, width);
        house::menu_inset(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            house::caps(ui, "Custom");
            ui.horizontal(|ui| {
                for c in load_custom() {
                    if swatch(ui, c, now == Some(c)) {
                        picked = Some(c);
                    }
                }
                let add = match crate::icons::icon("add_circle") {
                    Some(img) => egui::Button::image(img.fit_to_exact_size(vec2(20.0, 20.0))),
                    None => egui::Button::new("+"),
                };
                if ui.add(add.frame(false)).on_hover_text("Custom colour").clicked() {
                    choice = Some(Choice::Custom);
                }
            });
        });
    }
    if conditional {
        crate::shell::menu_sep_width(ui, width);
        if ui.add(egui::Button::new(text("Conditional formatting")).frame_when_inactive(false).min_size(vec2(0.0, house::ROW_HEIGHT))).clicked() {
            choice = Some(Choice::Conditional);
        }
    }
    if let Some(rgb) = picked {
        choice = Some(Choice::Colour(rgb));
    }
    choice
}

/// The toolbar's text and fill colour menus.
pub fn menu(app: &mut SheetApp, ui: &mut Ui, cmd: &str, reset_value: &str) {
    let now = current(app, cmd);
    menu_for(app, ui, cmd, json!({}), reset_value, now, true);
}

/// A colour menu that runs `cmd` with `params` plus `"color"` (e.g. a sheet tab's colour).
pub fn menu_for(app: &mut SheetApp, ui: &mut Ui, cmd: &str, params: serde_json::Value, reset_value: &str, now: Option<u32>, conditional: bool) {
    let with = |colour: String| {
        let mut p = params.clone();
        p["color"] = json!(colour);
        p
    };
    match palette(ui, now, true, conditional) {
        Some(Choice::Reset) => app.run_or_alert(cmd, with(reset_value.to_string())),
        Some(Choice::Colour(rgb)) => app.run_or_alert(cmd, with(format!("#{rgb:06X}"))),
        Some(Choice::Custom) => {
            let start = now.unwrap_or(0x000000);
            ui.ctx().data_mut(|m| m.insert_temp(egui::Id::unique(PICKER_REQUEST), (cmd.to_string(), start, params.to_string())));
        }
        Some(Choice::Conditional) => app.open_dialog("manageRules", json!({})),
        None => {}
    }
}

fn swatches(ui: &mut Ui, row: &[u32], now: Option<u32>, picked: &mut Option<u32>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for c in row {
            if swatch(ui, *c, now == Some(*c)) {
                *picked = Some(*c);
            }
        }
    });
}

/// One round swatch; true when clicked. The current colour gets a tick, as in Sheets.
fn swatch(ui: &mut Ui, rgb: u32, selected: bool) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(19.0, 19.0), Sense::click());
    let p = ui.painter();
    p.circle_filled(r.center(), 9.0, c32(rgb));
    if rgb > 0xE8E8E8 {
        p.circle_stroke(r.center(), 9.0, Stroke::new(1.0, theme::DIVIDER));
    }
    if resp.hovered() {
        p.circle_stroke(r.center(), 10.0, Stroke::new(1.5, theme::BLUE));
    }
    if selected {
        let light = ((rgb >> 16) & 0xFF) + ((rgb >> 8) & 0xFF) + (rgb & 0xFF) > 0x180;
        let tick = if light { Color32::BLACK } else { Color32::WHITE };
        let c = r.center();
        p.line(vec![c + vec2(-4.0, 0.0), c + vec2(-1.0, 3.0), c + vec2(4.5, -3.5)], Stroke::new(1.8, tick));
    }
    resp.on_hover_text(format!("#{rgb:06X}")).clicked()
}

fn load_custom() -> Vec<u32> {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .and_then(|s| s.get_item(CUSTOM_KEY).ok().flatten())
            .map(|t| t.split(',').filter_map(|h| u32::from_str_radix(h.trim(), 16).ok()).collect())
            .unwrap_or_default()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = CUSTOM_KEY;
        Vec::new()
    }
}

fn remember_custom(rgb: u32) {
    let mut list = load_custom();
    list.retain(|c| *c != rgb);
    list.insert(0, rgb);
    list.truncate(CUSTOM_MAX);
    #[cfg(target_arch = "wasm32")]
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item(CUSTOM_KEY, &list.iter().map(|c| format!("{c:06X}")).collect::<Vec<_>>().join(","));
    }
}

/// The ⊕ picker, open after the menu closed: saturation square, hue slider, Hex / R / G / B.
#[derive(Default)]
pub struct CustomPicker {
    state: Option<Picking>,
}

struct Picking {
    cmd: String,
    /// The command's other parameters (the colour is added to them).
    params: serde_json::Value,
    h: f32,
    s: f32,
    v: f32,
    hex: String,
    rgb_text: [String; 3],
}

impl Picking {
    fn rgb(&self) -> u32 {
        hsv_to_rgb(self.h, self.s, self.v)
    }

    fn set_rgb(&mut self, rgb: u32) {
        let (h, s, v) = rgb_to_hsv(rgb);
        // Keep the hue for greys (it has none), so the slider doesn't jump.
        if s > 0.0 && v > 0.0 {
            self.h = h;
        }
        self.s = s;
        self.v = v;
        self.sync_text();
    }

    fn sync_text(&mut self) {
        let rgb = self.rgb();
        self.hex = format!("#{rgb:06X}");
        self.rgb_text = [(rgb >> 16 & 0xFF).to_string(), (rgb >> 8 & 0xFF).to_string(), (rgb & 0xFF).to_string()];
    }
}

impl CustomPicker {
    pub fn ui(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        if let Some((cmd, start, params)) = ctx.data_mut(|m| m.remove_temp::<(String, u32, String)>(egui::Id::unique(PICKER_REQUEST))) {
            let params = serde_json::from_str(&params).unwrap_or_else(|_| json!({}));
            let mut p = Picking { cmd, params, h: 0.0, s: 0.0, v: 0.0, hex: String::new(), rgb_text: Default::default() };
            p.set_rgb(start);
            self.state = Some(p);
        }
        let Some(p) = self.state.as_mut() else { return };
        let mut done = None;
        let mut closed = false;
        house::dialog(ctx, "starsheets_custom_colour", 300.0, "Custom colour", Some(&mut closed), |ui| {
            // Saturation (across) and brightness (down) for the chosen hue.
            let (sq, resp) = ui.allocate_exact_size(vec2(300.0, 180.0), Sense::click_and_drag());
            paint_sv(ui, sq, p.h);
            if let Some(pos) = resp.interact_pointer_pos() {
                p.s = ((pos.x - sq.left()) / sq.width()).clamp(0.0, 1.0);
                p.v = 1.0 - ((pos.y - sq.top()) / sq.height()).clamp(0.0, 1.0);
                p.sync_text();
            }
            let knob = Pos2::new(sq.left() + p.s * sq.width(), sq.top() + (1.0 - p.v) * sq.height());
            ui.painter().circle(knob, 7.0, c32(p.rgb()), Stroke::new(2.0, Color32::WHITE));
            ui.add_space(house::M);
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 16.0, c32(p.rgb()));
                let (bar, hr) = ui.allocate_exact_size(vec2(300.0 - 34.0 - house::S, 12.0), Sense::click_and_drag());
                paint_hue(ui, bar);
                if let Some(pos) = hr.interact_pointer_pos() {
                    p.h = ((pos.x - bar.left()) / bar.width()).clamp(0.0, 1.0) * 360.0;
                    p.sync_text();
                }
                let x = bar.left() + p.h / 360.0 * bar.width();
                ui.painter().circle(egui::pos2(x, bar.center().y), 8.0, c32(hsv_to_rgb(p.h, 1.0, 1.0)), Stroke::new(2.0, Color32::WHITE));
            });
            ui.add_space(house::M);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    house::caps(ui, "Hex");
                    let r = ui.add(egui::TextEdit::singleline(&mut p.hex).desired_width(96.0).margin(vec2(house::S, house::S)));
                    if r.changed()
                        && let Some(rgb) = parse_hex(&p.hex)
                    {
                        let keep = p.hex.clone();
                        p.set_rgb(rgb);
                        p.hex = keep;
                    }
                });
                for (i, name) in ["R", "G", "B"].iter().enumerate() {
                    ui.vertical(|ui| {
                        house::caps(ui, name);
                        let r = ui.add(egui::TextEdit::singleline(&mut p.rgb_text[i]).desired_width(36.0).margin(vec2(house::S, house::S)));
                        if r.changed() {
                            let parts: Vec<u32> = p.rgb_text.iter().map(|t| t.trim().parse::<u32>().unwrap_or(0).min(255)).collect();
                            let keep = p.rgb_text.clone();
                            p.set_rgb(parts[0] << 16 | parts[1] << 8 | parts[2]);
                            p.rgb_text = keep;
                        }
                    });
                }
            });
            match house::actions(ui, &["Cancel", "OK"]) {
                Some(0) => done = Some(false),
                Some(_) => done = Some(true),
                None => {}
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = Some(false);
            }
        });
        if closed {
            done = Some(false);
        }
        match done {
            Some(true) => {
                let rgb = p.rgb();
                let cmd = p.cmd.clone();
                remember_custom(rgb);
                let mut params = p.params.clone();
                params["color"] = json!(format!("#{rgb:06X}"));
                app.run_or_alert(&cmd, params);
                self.state = None;
            }
            Some(false) => self.state = None,
            None => {}
        }
    }
}

/// The square: white → hue across, then black downwards (a mesh, so it's smooth).
fn paint_sv(ui: &Ui, r: Rect, h: f32) {
    let mut mesh = egui::Mesh::default();
    const N: usize = 16;
    for yi in 0..=N {
        for xi in 0..=N {
            let (s, v) = (xi as f32 / N as f32, 1.0 - yi as f32 / N as f32);
            let pos = egui::pos2(r.left() + s * r.width(), r.top() + (1.0 - v) * r.height());
            mesh.colored_vertex(pos, c32(hsv_to_rgb(h, s, v)));
        }
    }
    for yi in 0..N {
        for xi in 0..N {
            let a = (yi * (N + 1) + xi) as u32;
            let b = a + 1;
            let c = a + (N + 1) as u32;
            let d = c + 1;
            mesh.add_triangle(a, b, c);
            mesh.add_triangle(b, d, c);
        }
    }
    ui.painter().add(egui::Shape::mesh(mesh));
}

fn paint_hue(ui: &Ui, r: Rect) {
    let mut mesh = egui::Mesh::default();
    const N: usize = 36;
    for i in 0..=N {
        let t = i as f32 / N as f32;
        let colour = c32(hsv_to_rgb(t * 360.0, 1.0, 1.0));
        let x = r.left() + t * r.width();
        mesh.colored_vertex(egui::pos2(x, r.top()), colour);
        mesh.colored_vertex(egui::pos2(x, r.bottom()), colour);
    }
    for i in 0..N as u32 {
        let a = i * 2;
        mesh.add_triangle(a, a + 1, a + 2);
        mesh.add_triangle(a + 1, a + 3, a + 2);
    }
    ui.painter().add(egui::Shape::mesh(mesh));
}

fn parse_hex(s: &str) -> Option<u32> {
    let t = s.trim().trim_start_matches('#');
    (t.len() == 6).then(|| u32::from_str_radix(t, 16).ok()).flatten()
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> u32 {
    let c = v * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let to = |f: f32| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u32;
    to(r) << 16 | to(g) << 8 | to(b)
}

pub fn rgb_to_hsv(rgb: u32) -> (f32, f32, f32) {
    let (r, g, b) = ((rgb >> 16 & 0xFF) as f32 / 255.0, (rgb >> 8 & 0xFF) as f32 / 255.0, (rgb & 0xFF) as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, if max == 0.0 { 0.0 } else { d / max }, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_round_trip_through_hsv() {
        for rgb in [0x000000, 0xFFFFFF, 0xFF0000, 0x4285F4, 0x1C4587, 0xFBBC04, 0x7F6000] {
            let (h, s, v) = rgb_to_hsv(rgb);
            assert_eq!(hsv_to_rgb(h, s, v), rgb, "{rgb:06X}");
        }
        assert_eq!(parse_hex("#1a73e8"), Some(0x1A73E8));
        assert_eq!(parse_hex("12"), None);
    }
}
