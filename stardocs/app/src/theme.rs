//! StarDocs' interface theme and the finishing touches to WordCraft's drawing.
//! The browser follows the Starbase setting. Native StarDocs uses light chrome. The document's
//! own page appearance is a separate WordCraft choice.

use std::sync::Arc;

use egui::{Color32, CornerRadius, Order, Stroke, vec2};
use wordcraft_ui_egui::WordApp;
use wordcraft_ui_egui::theme::{Appearance, Tokens};

/// Google Docs' page surround (and the strip behind the rulers): the top bar's near-white.
pub const SURROUND: Color32 = Color32::from_rgb(0xF9, 0xFB, 0xFD);
pub(crate) struct Palette {
    pub page: Color32,
    pub toolbar: Color32,
    pub card: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub meta: Color32,
    pub divider: Color32,
    pub menu_line: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub menu_hover: Color32,
    pub active: Color32,
    pub share: Color32,
    pub accent: Color32,
    pub notice: Color32,
    pub error: Color32,
}

const fn rgb(colour: u32) -> Color32 {
    Color32::from_rgb((colour >> 16) as u8, (colour >> 8) as u8, colour as u8)
}

static LIGHT: Palette = Palette {
    page: SURROUND,
    toolbar: rgb(0xedf2fa),
    card: Color32::WHITE,
    text: rgb(0x1f1f1f),
    text_dim: rgb(0x444746),
    meta: rgb(0x5f6368),
    divider: rgb(0xc7c7c7),
    menu_line: rgb(0xdadce0),
    hover: rgb(0xdde3ea),
    pressed: rgb(0xc9d3df),
    menu_hover: rgb(0xf1f3f4),
    active: rgb(0xd3e3fd),
    share: rgb(0xc2e7ff),
    accent: rgb(0x0b57d0),
    notice: rgb(0x8a5a00),
    error: rgb(0xb3261e),
};

#[cfg(target_arch = "wasm32")]
static DARK: Palette = Palette {
    page: rgb(0x1b1b1b),
    toolbar: rgb(0x2d2f31),
    card: rgb(0x2d2f31),
    text: rgb(0xe3e3e3),
    text_dim: rgb(0xc4c7c5),
    meta: rgb(0x9aa0a6),
    divider: rgb(0x5f6368),
    menu_line: rgb(0x44474a),
    hover: rgb(0x3c4043),
    pressed: rgb(0x4a4e51),
    menu_hover: rgb(0x3c4043),
    active: rgb(0x004a77),
    share: rgb(0x2b4465),
    accent: rgb(0xa8c7fa),
    notice: rgb(0xf6c35b),
    error: rgb(0xf2b8b5),
};

#[cfg(target_arch = "wasm32")]
thread_local! {
    static BROWSER_DARK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn browser_dark() -> bool {
    BROWSER_DARK.with(std::cell::Cell::get)
}

pub(crate) fn palette() -> &'static Palette {
    #[cfg(target_arch = "wasm32")]
    if browser_dark() {
        return &DARK;
    }
    &LIGHT
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn set_browser_dark(app: &mut WordApp, dark: bool) {
    app.ui.theme = if dark { Appearance::Dark } else { Appearance::Light };
    if BROWSER_DARK.with(|current| current.replace(dark)) != dark {
        crate::looks::apply(app);
    }
}

/// Tooltips: dark grey with white text, 4 px corners (Google's look, as StarSheets has).
pub const TIP_BG: Color32 = Color32::from_rgb(0x3C, 0x40, 0x43);
/// The picked row in a dark pop-up on the tooltip layer (white text stays readable on it).
const TIP_PICKED: Color32 = Color32::from_rgb(0x0B, 0x57, 0xD0);
/// A hovered row in a dark pop-up.
const TIP_HOVER: Color32 = Color32::from_rgb(0x5F, 0x63, 0x68);

/// How many surround shapes the last frame repainted (for [`state`]).
fn drawn_id() -> egui::Id {
    egui::Id::unique("stardocs_surround_drawn")
}

/// Runs [`after_pass`] at the end of every frame of `ctx`, once however often it is called.
pub fn install(ctx: &egui::Context) {
    let id = egui::Id::unique("stardocs_looks_installed");
    if ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false) {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(id, true));
    ctx.on_end_pass("stardocs_looks", Arc::new(|ui: &mut egui::Ui| after_pass(ui.ctx())));
}

/// WordCraft paints the area round the page and behind the rulers with its `canvas` colour
/// (#E6E6E6), which isn't a setting; Docs' is [`SURROUND`]. Its square-cornered fills of that
/// colour on the window's background layer are repainted. (Hovered buttons share the colour but
/// have rounded corners, so they stay.) Tooltips are restyled dark, as in StarSheets: egui draws
/// them in the menu frame, so only the tooltip layer is changed.
fn after_pass(ctx: &egui::Context) {
    let tokens = Tokens::get(ctx);
    let canvas = tokens.canvas;
    let shadow = tokens.page_shadow;
    let tip_fill = ctx.global_style().visuals.window_fill;
    let mut layers: Vec<egui::LayerId> =
        ctx.memory(|m| m.layer_ids().filter(|l| matches!(l.order, Order::Background | Order::Middle | Order::Foreground | Order::Tooltip)).collect());
    if !layers.contains(&egui::LayerId::background()) {
        layers.push(egui::LayerId::background());
    }
    let mut drawn = 0_usize;
    ctx.graphics_mut(|g| {
        for l in layers {
            let Some(list) = g.get_mut(l) else { continue };
            for i in 0..list.next_idx().0 {
                list.mutate_shape(egui::layers::ShapeIdx(i), |cs| {
                    match l.order {
                        Order::Tooltip => {
                            if restyle_tip(&mut cs.shape, tip_fill) {
                                cs.clip_rect = cs.clip_rect.expand2(vec2(8.0, 4.0));
                            }
                        }
                        Order::Background => {
                            if !tokens.dark {
                                drawn += recolour_surround(&mut cs.shape, canvas);
                                soften_page_shadow(&mut cs.shape, shadow);
                            }
                        }
                        // Dialogs and pop-ups: Google's small grey drop-down arrow.
                        _ => {
                            material_arrow(&mut cs.shape);
                            if !tokens.dark {
                                soften_page_shadow(&mut cs.shape, shadow);
                            }
                        }
                    }
                });
            }
        }
    });
    ctx.data_mut(|d| d.insert_temp(drawn_id(), drawn));
}

/// Repaints WordCraft's square `canvas`-coloured fills [`SURROUND`]; how many it changed.
fn recolour_surround(shape: &mut egui::Shape, canvas: Color32) -> usize {
    match shape {
        egui::Shape::Rect(r) if r.fill == canvas && r.corner_radius == CornerRadius::ZERO => {
            r.fill = SURROUND;
            1
        }
        egui::Shape::Vec(v) => v.iter_mut().map(|s| recolour_surround(s, canvas)).sum(),
        _ => 0,
    }
}

/// WordCraft draws a page's shadow as a hard 2 px offset rectangle; Docs' is a faint, soft glow
/// round the page. Blurs that rectangle (found by its colour) and thins it.
fn soften_page_shadow(shape: &mut egui::Shape, shadow: Color32) {
    match shape {
        egui::Shape::Rect(r) if r.fill == shadow && r.corner_radius == CornerRadius::same(1) => {
            r.fill = Color32::from_black_alpha(0x1C);
            r.blur_width = 6.0;
        }
        egui::Shape::Vec(v) => v.iter_mut().for_each(|s| soften_page_shadow(s, shadow)),
        _ => {}
    }
}

/// egui draws a drop-down's arrow as a big filled triangle in the text colour (WordCraft's
/// dialogs: Paper size, Vertical alignment …); Google's is a small grey one. Each downward
/// triangle of a drop-down's size becomes Google's, about the same centre.
fn material_arrow(shape: &mut egui::Shape) {
    match shape {
        egui::Shape::Path(p) if p.closed && p.points.len() == 3 && p.fill.a() > 0 => {
            let [a, b, c] = [p.points[0], p.points[1], p.points[2]];
            let w = (b.x - a.x).abs();
            let down = (a.y - b.y).abs() < 0.5 && (c.x - (a.x + b.x) / 2.0).abs() < 0.5 && c.y > a.y;
            if down && (6.0..=24.0).contains(&w) {
                let centre = egui::pos2((a.x + b.x) / 2.0, (a.y + c.y) / 2.0);
                p.points = vec![centre + vec2(-5.0, -2.5), centre + vec2(5.0, -2.5), centre + vec2(0.0, 2.5)];
                p.fill = palette().text_dim;
            }
        }
        egui::Shape::Vec(v) => v.iter_mut().for_each(material_arrow),
        _ => {}
    }
}

/// Restyles one tooltip shape; true when it grew (the background, given side padding).
fn restyle_tip(shape: &mut egui::Shape, fill: Color32) -> bool {
    match shape {
        egui::Shape::Rect(r) if r.fill == fill => {
            r.fill = TIP_BG;
            r.stroke = Stroke::NONE;
            r.corner_radius = CornerRadius::same(4);
            r.rect = r.rect.expand2(vec2(8.0, 0.0));
            true
        }
        // Rows inside dark pop-ups on the tooltip layer: a light highlight under white text
        // can't be read, so a picked row turns strong blue and a hovered one mid grey.
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

/// Pins WordCraft's interface theme to Light. WordCraft applies it (egui `ThemePreference::Light`)
/// in its next `logic` pass, so every egui surface takes the light visuals.
#[cfg(not(target_arch = "wasm32"))]
pub fn keep_light(app: &mut WordApp) {
    app.ui.theme = Appearance::Light;
}

/// What the test hook reports (`state().theme`): the system's preference, the theme egui draws
/// with, and the colours of the page surround (as drawn: [`SURROUND`] once [`after_pass`] has
/// repainted it, with how many shapes it repainted), the rulers, a hovered button, a hovered and
/// a pressed toolbar button and the tooltips, as `#rrggbb`.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn state(ctx: &egui::Context) -> serde_json::Value {
    let name = |t: egui::Theme| if t == egui::Theme::Dark { "dark" } else { "light" };
    let hex = |c: egui::Color32| format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b());
    let tokens = Tokens::get(ctx);
    let visuals = ctx.global_style().visuals.clone();
    let drawn = ctx.data(|d| d.get_temp::<usize>(drawn_id())).unwrap_or(0);
    serde_json::json!({
        "system": ctx.system_theme().map(name),
        "interface": name(ctx.theme()),
        "surround": hex(if drawn > 0 { SURROUND } else { tokens.canvas }),
        "surroundShapes": drawn,
        "toolbarHover": hex(palette().hover),
        "toolbarPressed": hex(palette().pressed),
        "tooltip": hex(TIP_BG),
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

    /// The area round the page and behind the rulers is Docs' near-white, not WordCraft's grey.
    #[test]
    fn the_page_surround_is_docs_near_white() {
        let h = dark_system_window(new_app());
        let st = super::state(&h.ctx);
        assert_eq!(st["surround"], "#f9fbfd", "{st}");
        assert!(st["surroundShapes"].as_u64().unwrap() >= 1, "{st}");
    }

    /// Tooltips are dark with white text and no border, as in StarSheets and Google.
    #[test]
    fn tooltips_are_dark() {
        use egui::epaint::{CornerRadius, RectShape, Stroke as S};
        let fill = Color32::WHITE;
        let bg = RectShape::new(
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(50.0, 20.0)),
            CornerRadius::same(6),
            fill,
            S::new(1.0, Color32::GRAY),
            egui::StrokeKind::Inside,
        );
        let mut shape = egui::Shape::Vec(vec![egui::Shape::Rect(bg)]);
        assert!(super::restyle_tip(&mut shape, fill), "the background grows");
        let egui::Shape::Vec(v) = shape else { panic!() };
        let egui::Shape::Rect(r) = &v[0] else { panic!() };
        assert_eq!((r.fill, r.stroke, r.corner_radius), (super::TIP_BG, S::NONE, CornerRadius::same(4)));
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
