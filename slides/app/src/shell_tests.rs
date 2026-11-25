//! Native tests of the window (egui_kittest's accessibility tree, no GPU): our menus and toolbar
//! around DeckCraft's slides, with DeckCraft's own title bar, ribbon and status bar off.

use std::cell::RefCell;
use std::rc::Rc;

use deckcraft_engine::Session;
use deckcraft_ui_egui::{Services, SlideApp};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::shell::{Drive, Saved, Shell};
use crate::theme::Mode;

struct State {
    app: SlideApp,
    shell: Shell,
}

fn app_with(services: Services) -> SlideApp {
    let mut app = SlideApp::new(Session::new(), services).with_chrome(crate::shell::chrome());
    app.run("file.new", json!({})).unwrap_or_default();
    app
}

fn harness_with(app: SlideApp, mode: Mode) -> Harness<'static, State> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
        |ui, s: &mut State| {
            s.app.logic(ui.ctx());
            s.shell.ui(&mut s.app, ui, None);
            s.app.ui(ui);
        },
        State { app, shell: Shell::new(mode) },
    );
    egui_extras::install_image_loaders(&h.ctx);
    h.run_steps(4);
    h
}

fn harness() -> Harness<'static, State> {
    harness_with(app_with(Services::default()), Mode::Light)
}

fn slides(app: &SlideApp) -> usize {
    app.session.active().map_or(0, |d| d.doc.slides.len())
}

#[test]
fn every_menu_command_exists() {
    for id in crate::shell::menu_ids() {
        assert!(id.starts_with("slides.") || deckcraft_ui_egui::ribbon::known(id), "menu command `{id}` doesn't exist in DeckCraft");
    }
}

#[test]
fn every_icon_the_window_uses_exists() {
    let src = include_str!("shell.rs");
    let mut missing = Vec::new();
    for (i, _) in src.match_indices("icon_button(ui, \"").chain(src.match_indices("tool(ui, pal, \"")) {
        let rest = &src[i..];
        let start = rest.find('"').unwrap_or(0) + 1;
        let name = &rest[start..start + rest[start..].find('"').unwrap_or(0)];
        if !crate::icons::has(name) {
            missing.push(name.to_string());
        }
    }
    for name in ["add", "undo", "redo", "slideshow", "check", "arrow_right", "comment", "shapes", "text_fields", "arrow_selector_tool"] {
        if !crate::icons::has(name) {
            missing.push(name.to_string());
        }
    }
    assert!(missing.is_empty(), "icons not in icons.rs: {missing:?}");
}

#[test]
fn shape_presets_in_the_menus_exist() {
    for p in ["rect", "roundRect", "ellipse", "triangle", "rightArrow", "star5", "line"] {
        assert!(deckcraft_geom_has(p), "no shape preset `{p}`");
    }
}

fn deckcraft_geom_has(preset: &str) -> bool {
    let mut app = app_with(Services::default());
    app.session.execute("shape.insert", &json!({"preset": preset, "rect": [40, 40, 120, 80]})).is_ok()
}

#[test]
fn our_menus_replace_deckcrafts_title_bar_ribbon_and_status_bar() {
    let h = harness();
    for menu in ["File", "Edit", "View", "Insert", "Format", "Slide", "Arrange", "Help"] {
        assert!(h.query_by_label(menu).is_some(), "menu {menu}");
    }
    assert!(h.query_by_label("Slideshow").is_some(), "Slideshow button");
    assert!(!h.query_all_by_label_contains("Failed loading").any(|_| true), "every picture loads");
    assert!(h.query_by_label("New slide (Ctrl+M)").is_some(), "toolbar");
    assert!(h.query_by_label("Window title bar").is_none(), "DeckCraft's title bar is off");
    assert!(h.query_by_label("Animations").is_none(), "DeckCraft's ribbon is off");
    assert!(h.query_by_label("Zoom level").is_none(), "DeckCraft's status bar is off");
}

#[test]
fn the_toolbar_runs_deckcraft_commands() {
    let mut h = harness();
    let before = slides(&h.state().app);
    h.get_by_label("New slide (Ctrl+M)").click();
    h.run_steps(3);
    assert_eq!(slides(&h.state().app), before + 1, "New slide added one");
    h.get_by_label("Undo (Ctrl+Z)").click();
    h.run_steps(3);
    assert_eq!(slides(&h.state().app), before, "Undo took it away");
}

#[test]
fn our_commands_set_the_drawing_tool() {
    let mut h = harness();
    let s = h.state_mut();
    s.shell.command(&mut s.app, None, "slides.shape", json!({"preset": "ellipse"})).unwrap_or_default();
    assert!(matches!(&s.app.session.tool.kind, deckcraft_engine::ToolKind::Shape { preset } if preset == "ellipse"));
    s.shell.command(&mut s.app, None, "slides.textBox", json!({})).unwrap_or_default();
    assert!(matches!(s.app.session.tool.kind, deckcraft_engine::ToolKind::TextBox));
    assert!(s.shell.command(&mut s.app, None, "slides.nope", json!({})).is_err(), "unknown own command");
}

#[test]
fn light_is_the_default_and_dark_still_switches_both_halves() {
    assert_eq!(Mode::default(), Mode::Light, "light-only like Drive, Sheets and Docs");
    assert_eq!(Mode::parse("system"), Mode::Light);
    let mut h = harness();
    assert!(!h.state().app.ui.brightness.eq(&deckcraft_ui_egui::theme::Brightness::Dark), "light to start");
    {
        let s = h.state_mut();
        s.shell.command(&mut s.app, None, "slides.theme", json!({"mode": "dark"})).unwrap_or_default();
    }
    h.run_steps(3);
    assert_eq!(h.state().shell.mode, Mode::Dark);
    assert_eq!(h.state().app.ui.brightness, deckcraft_ui_egui::theme::Brightness::Dark, "DeckCraft's half is dark too");
    assert!(h.ctx.global_style().visuals.dark_mode, "egui restyled dark");
    {
        let s = h.state_mut();
        s.shell.command(&mut s.app, None, "slides.theme", json!({"mode": "light"})).unwrap_or_default();
    }
    h.run_steps(3);
    assert_eq!(h.state().app.ui.brightness, deckcraft_ui_egui::theme::Brightness::Light);
    assert!(!h.ctx.global_style().visuals.dark_mode);
}

#[test]
fn download_hands_over_a_pptx() {
    let got: Rc<RefCell<Vec<(String, Vec<u8>)>>> = Rc::default();
    let keep = got.clone();
    let services = Services {
        download: Some(Box::new(move |name: &str, bytes: &[u8]| keep.borrow_mut().push((name.to_string(), bytes.to_vec())))),
        ..Default::default()
    };
    let mut h = harness_with(app_with(services), Mode::Light);
    let s = h.state_mut();
    let r = s.shell.command(&mut s.app, None, "slides.download", json!({"format": "pptx"}));
    assert!(r.is_ok(), "{r:?}");
    let got = got.borrow();
    assert_eq!(got.len(), 1);
    assert!(got[0].0.ends_with(".pptx"), "{}", got[0].0);
    assert!(got[0].1.starts_with(b"PK"), "a zip package");
}

/// A pretend server link, to see the title row and what a viewer may do.
struct FakeDrive {
    view_only: bool,
}

impl Drive for FakeDrive {
    fn name(&self) -> String {
        "Quarterly plan.pptx".into()
    }
    fn saved(&self) -> Saved {
        Saved::Done
    }
    fn notice(&self) -> Option<String> {
        None
    }
    fn view_only(&self) -> bool {
        self.view_only
    }
    fn home(&self) {}
    fn new_presentation(&self) {}
    fn rename(&self, _stem: &str) {}
}

fn harness_on_server(view_only: bool) -> Harness<'static, (State, FakeDrive)> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
        |ui, (s, d): &mut (State, FakeDrive)| {
            s.app.logic(ui.ctx());
            s.shell.ui(&mut s.app, ui, Some(&*d));
            s.app.ui(ui);
        },
        (State { app: app_with(Services::default()), shell: Shell::new(Mode::Light) }, FakeDrive { view_only }),
    );
    egui_extras::install_image_loaders(&h.ctx);
    h.run_steps(4);
    h
}

#[test]
fn the_title_row_shows_the_server_name_without_pptx_and_the_save_state() {
    let h = harness_on_server(false);
    assert!(h.query_by_label("Quarterly plan").is_some(), "name without .pptx");
    assert!(h.query_by_label("Saved to Starbase").is_some());
}

#[test]
fn a_viewer_gets_a_greyed_toolbar() {
    let h = harness_on_server(true);
    assert!(h.query_by_label("View only").is_some());
    let new_slide = h.get_by_label("New slide (Ctrl+M)");
    assert!(new_slide.accesskit_node().is_disabled(), "New slide is greyed for a viewer");
}

#[test]
fn the_default_slide_font_is_in_the_program() {
    crate::fonts::add();
    let families = crate::fonts::families();
    assert!(families.iter().any(|f| f == "Inter"), "Inter is bundled: {families:?}");
    let mut app = app_with(Services::default());
    let f = app.session.execute("format.state", &json!({})).unwrap_or_default();
    assert_eq!(f["font"], "Inter", "a new presentation's text is Inter");
    let more = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fonts/more");
    assert!(crate::fonts::MORE.iter().all(|f| more.join(f).is_file()), "every fetched face is in fonts/more");
}

#[test]
fn presenter_view_asks_for_the_presenter_window_for_one_show() {
    let mut h = harness();
    {
        let s = h.state_mut();
        s.app.ui.presenter_view = false;
        s.shell.command(&mut s.app, None, "slides.presenter", json!({})).unwrap_or_default();
        assert!(s.shell.presenter_flag().get(), "the presenter window is asked for");
        assert!(s.app.ui.presenter_view);
    }
    h.run_steps(3);
    assert!(h.state().app.show.is_some(), "the show runs");
    let _ = h.state_mut().app.run("show.end", json!({}));
    h.run_steps(3);
    assert!(h.state().app.show.is_none());
    assert!(!h.state().shell.presenter_flag().get(), "only for that show: Slideshow shows the slides alone again");
}
