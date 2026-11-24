//! The window's behaviour, tested natively without a browser: our `Shell` over WordCraft's page,
//! driven through egui_kittest's accessibility tree (no GPU, no pictures).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "tests")]

use egui::{Event, Key, Modifiers, accesskit::Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{By, Queryable};
use serde_json::{Value, json};
use wordcraft_engine::Session;
use wordcraft_ui_egui::{HostChrome, Services, WordApp};

use crate::shell::Shell;

/// A blank Docs-style document in our window, as web.rs sets it up (no server, no host).
struct Win {
    app: WordApp,
    shell: Shell,
}

impl Win {
    fn state(&mut self) -> Value {
        self.shell.test_state(&mut self.app, None)
    }
    fn view(&mut self, key: &str) -> Value {
        self.app.run("view.state", json!({})).unwrap()[key].clone()
    }
}

fn window(width: f32) -> Harness<'static, Win> {
    let mut doc = wordcraft_engine::blank_document(&Default::default());
    crate::shell::docs_defaults(&mut doc);
    let mut app = WordApp::new(Session::new(doc), Services::default()).with_chrome(HostChrome { title_bar: false, ribbon: false, status_bar: false });
    app.ui.language = "en".into();
    app.autosave = false;
    let win = Win { app, shell: Shell::default() };
    let mut h = Harness::builder().with_size(egui::vec2(width, 900.0)).build_ui_state(
        |ui, w: &mut Win| {
            egui_extras::install_image_loaders(ui.ctx());
            w.app.logic(ui.ctx());
            w.shell.ui(&mut w.app, ui, None);
            w.app.ui(ui);
        },
        win,
    );
    steps(&mut h, 6);
    h
}

/// Runs frames, passing input through WordCraft's raw input hook as the web runner does.
fn steps(h: &mut Harness<'static, Win>, n: usize) {
    for _ in 0..n {
        let mut raw = std::mem::take(h.input_mut());
        let ctx = h.ctx.clone();
        h.state_mut().app.raw_input_hook(&ctx, &mut raw);
        *h.input_mut() = raw;
        h.step();
    }
}

/// Clicks the control named `label` exactly.
fn click(h: &mut Harness<'static, Win>, label: &str) {
    h.get(By::new().label(label)).click();
    steps(h, 3);
}

/// Clicks the one control whose name contains `part` (a menu tick, a two-line item).
fn click_containing(h: &mut Harness<'static, Win>, part: &str) {
    h.get(By::new().label_contains(part)).click();
    steps(h, 3);
}

fn keys(h: &mut Harness<'static, Win>, modifiers: Modifiers, key: Key) {
    h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    steps(h, 1);
    h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
    steps(h, 2);
}

fn type_text(h: &mut Harness<'static, Win>, text: &str) {
    h.input_mut().events.push(Event::Text(text.into()));
    steps(h, 3);
}

fn body_text(h: &mut Harness<'static, Win>) -> String {
    h.state().app.session.doc.plain_text(wordcraft_doc::StoryRef::Body)
}

#[test]
fn view_show_outline_turns_the_outline_on_and_off() {
    let mut h = window(1400.0);
    assert_eq!(h.state_mut().view("navPane"), false);
    click(&mut h, "View");
    click_containing(&mut h, "Show outline");
    assert_eq!(h.state_mut().view("navPane"), true);
    click(&mut h, "View");
    click_containing(&mut h, "Show outline");
    assert_eq!(h.state_mut().view("navPane"), false);
}

#[test]
fn insert_table_grid_inserts_a_table() {
    let mut h = window(1400.0);
    click(&mut h, "Insert");
    click(&mut h, "Table");
    click(&mut h, "Table size");
    let doc = &h.state().app.session.doc;
    let tables = doc.story(wordcraft_doc::StoryRef::Body).unwrap().iter().filter(|b| b.as_table().is_some()).count();
    assert_eq!(tables, 1, "the grid inserted one table");
}

#[test]
fn mode_box_switches_to_suggesting() {
    let mut h = window(1400.0);
    assert_eq!(h.state_mut().state()["mode"], "editing");
    click(&mut h, "Editing");
    click_containing(&mut h, "Edits become suggestions");
    assert!(h.state().app.session.doc.settings.track_changes);
    assert_eq!(h.state_mut().state()["mode"], "suggesting");
}

#[test]
fn shortcuts_toggle_compact_and_open_the_shortcuts_list() {
    let mut h = window(1400.0);
    assert_eq!(h.state_mut().state()["compact"], false);
    keys(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::F);
    assert_eq!(h.state_mut().state()["compact"], true);
    keys(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::F);
    assert_eq!(h.state_mut().state()["compact"], false);
    keys(&mut h, Modifiers::COMMAND, Key::Slash);
    assert_eq!(h.state_mut().state()["dialogs"]["shortcuts"], true);
}

#[test]
fn toolbar_moves_groups_into_the_more_menu_when_narrow() {
    // The nine groups measure about 1300 px together, plus the mode box and the margins.
    let mut wide = window(1600.0);
    assert_eq!(wide.state_mut().state()["toolbar"]["groupsInMoreMenu"], 0, "{}", wide.state_mut().state()["toolbar"]);
    let mut narrow = window(900.0);
    let more = narrow.state_mut().state()["toolbar"]["groupsInMoreMenu"].as_u64().unwrap();
    assert!(more > 0, "at 900 px some groups are in the ⋮ menu");
    assert!(narrow.query(By::new().role(Role::Button).label("More")).is_some(), "the ⋮ button is shown");
}

#[test]
fn own_commands_run_through_the_test_hook() {
    let mut h = window(1400.0);
    let Win { app, shell } = h.state_mut();
    assert_eq!(shell.test_command(app, None, "stardocs.words", json!({})), Ok(json!(true)));
    assert!(shell.test_command(app, None, "format.bold", json!({})).is_err(), "only StarDocs' own commands");
    steps(&mut h, 2);
    assert_eq!(h.state_mut().state()["wordCountShown"], true);
}

#[test]
fn bold_button_makes_typed_text_bold() {
    let mut h = window(1400.0);
    click(&mut h, "Bold (Ctrl+B)");
    type_text(&mut h, "strong");
    let f = h.state_mut().app.run("format.state", json!({})).unwrap();
    assert_eq!(f["bold"], true, "{f}");
}

#[test]
fn toolbar_click_gives_the_keyboard_back_to_the_page() {
    let mut h = window(1400.0);
    click(&mut h, "Italic (Ctrl+I)");
    type_text(&mut h, "typed after a toolbar click");
    assert!(body_text(&mut h).contains("typed after a toolbar click"), "the page has the keyboard");
}
