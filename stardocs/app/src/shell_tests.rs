//! The window's behaviour, tested natively without a browser: our `Shell` over WordCraft's page,
//! driven through egui_kittest's accessibility tree (no GPU, no pictures).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "tests")]

use egui::{Event, Key, Modifiers, accesskit::Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{By, NodeT, Queryable};
use serde_json::{Value, json};
use wordcraft_engine::Session;
use wordcraft_ui_egui::{HostChrome, Services, WordApp};

use crate::shell::Shell;

/// A blank Docs-style document in our window, as web.rs sets it up (no server, no host).
struct Win {
    app: WordApp,
    shell: Shell,
    /// Where the buttons are with no menu open: the toolbar's (see [`entry`]).
    toolbar: Vec<egui::Rect>,
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
    let win = Win { app, shell: Shell::default(), toolbar: Vec::new() };
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
    let toolbar = h.query_all(By::new().role(Role::Button)).map(|n| n.rect()).collect();
    h.state_mut().toolbar = toolbar;
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
fn toolbar_fits_at_1400_and_moves_groups_into_the_more_menu_when_narrower() {
    // Like Google Docs, every group shows at 1400 px; narrower windows put the last ones in ⋮.
    let mut wide = window(1400.0);
    let t = wide.state_mut().state()["toolbar"].clone();
    assert_eq!(t["groupsInMoreMenu"], 0, "{t}");
    assert!(wide.query(By::new().role(Role::Button).label("More")).is_none(), "no ⋮ button at 1400 px");
    for width in [1100.0, 900.0] {
        let mut narrow = window(width);
        let more = narrow.state_mut().state()["toolbar"]["groupsInMoreMenu"].as_u64().unwrap();
        assert!(more > 0, "at {width} px some groups are in the ⋮ menu");
        assert!(narrow.query(By::new().role(Role::Button).label("More")).is_some(), "the ⋮ button is shown");
    }
}

fn font_size(h: &mut Harness<'static, Win>) -> Value {
    h.state_mut().app.run("format.state", json!({})).unwrap()["size"].clone()
}

#[test]
fn typing_in_the_font_size_box_sets_the_size() {
    let mut h = window(1400.0);
    assert_eq!(font_size(&mut h), json!(11.0));
    h.get(By::new().label("Font size")).focus();
    steps(&mut h, 3);
    assert!(h.query(By::new().label("14")).is_some(), "the size list opens under the box");
    type_text(&mut h, "13");
    keys(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(font_size(&mut h), json!(13.0));
    assert!(h.query(By::new().label("14")).is_none(), "Enter closes the list");
    type_text(&mut h, "after");
    assert!(body_text(&mut h).contains("after"), "the page has the keyboard again");
    // Out of range: nothing changes.
    h.get(By::new().label("Font size")).focus();
    steps(&mut h, 3);
    type_text(&mut h, "999");
    keys(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(font_size(&mut h), json!(13.0));
}

#[test]
fn the_font_size_list_still_works() {
    let mut h = window(1400.0);
    h.get(By::new().label("Font size")).focus();
    steps(&mut h, 3);
    click(&mut h, "18");
    assert_eq!(font_size(&mut h), json!(18.0));
}

#[test]
fn own_commands_run_through_the_test_hook() {
    let mut h = window(1400.0);
    let Win { app, shell, .. } = h.state_mut();
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

// Menu entries for WordCraft commands (gap list batch 1). Each test opens the menus as a person
// would and checks the document changed.

/// The open menu's entry worded `label`. A menu entry is named "wording shortcut", and some
/// toolbar buttons share Docs' wording ("Increase font size"), so buttons already on screen
/// before any menu opened don't count.
fn entry<'a>(h: &'a Harness<'static, Win>, label: &str) -> egui_kittest::Node<'a> {
    let toolbar = &h.state().toolbar;
    let named = |n: &egui_kittest::Node<'_>| {
        let l = n.accesskit_node().label().unwrap_or_default();
        let l = l.trim();
        l == label
            || l.strip_prefix(label)
                .and_then(|rest| rest.strip_prefix(' '))
                .is_some_and(|keys| ["Ctrl", "Alt", "Shift"].iter().any(|k| keys.starts_with(k)))
    };
    let mut found: Vec<_> = h.query_all(By::new().role(Role::Button)).filter(|n| named(n) && !toolbar.contains(&n.rect())).collect();
    assert_eq!(found.len(), 1, "one menu entry worded {label:?}");
    found.pop().unwrap()
}

/// Opens `path` one menu level at a time ("Format", "Table", …), clicking the last entry too.
fn menu(h: &mut Harness<'static, Win>, path: &[&str]) {
    click(h, path[0]);
    for label in &path[1..] {
        entry(h, label).click();
        steps(h, 3);
    }
}

/// Whether the open menu's entry worded `label` is greyed out.
fn is_greyed(h: &Harness<'static, Win>, label: &str) -> bool {
    entry(h, label).accesskit_node().is_disabled()
}

fn close_menus(h: &mut Harness<'static, Win>) {
    keys(h, Modifiers::NONE, Key::Escape);
}

fn run(h: &mut Harness<'static, Win>, id: &str, p: Value) {
    h.state_mut().app.run(id, p).unwrap();
    steps(h, 2);
}

/// The first table's (rows, columns, a row pinned as a header).
fn table_shape(h: &Harness<'static, Win>) -> Option<(usize, usize, bool)> {
    let doc = &h.state().app.session.doc;
    let t = doc.story(wordcraft_doc::StoryRef::Body).unwrap().iter().find_map(|b| b.as_table())?;
    Some((t.rows.len(), t.grid.len(), t.rows.iter().any(|r| r.props.header)))
}

fn para(h: &Harness<'static, Win>) -> wordcraft_doc::resolve::ResolvedPara {
    wordcraft_engine::cmd::para::selection_para(&h.state().app.session)
}

#[test]
fn every_menu_command_is_a_wordcraft_or_stardocs_command() {
    use crate::shell::Item;
    fn walk(items: &[Item], s: &Session, bad: &mut Vec<String>) {
        for item in items {
            match item {
                Item::Cmd(_, id, _, _) | Item::Toggle(_, id, _, _) | Item::Flip(_, _, id, _) => {
                    // WordCraft's window commands (not in the engine's registry) the menus use.
                    let window = matches!(*id, "ui.crop" | "ui.changePicture");
                    if !(window || id.starts_with("stardocs.") || id.starts_with("ui.dialog:") || s.registry.get(id).is_some()) {
                        bad.push((*id).to_string());
                    }
                }
                Item::Sub(_, sub) => walk(sub, s, bad),
                _ => {}
            }
        }
    }
    let h = window(1400.0);
    let mut bad = Vec::new();
    for (_, items) in crate::shell::MENUS {
        walk(items, &h.state().app.session, &mut bad);
    }
    assert!(bad.is_empty(), "unknown commands in the menus: {bad:?}");
}

#[test]
fn format_table_is_greyed_outside_a_table_and_edits_the_table_inside_one() {
    let mut h = window(1400.0);
    menu(&mut h, &["Format", "Table"]);
    for label in ["Insert row above", "Delete table", "Merge cells", "Table properties"] {
        assert!(is_greyed(&h, label), "{label} is greyed outside a table");
    }
    close_menus(&mut h);
    run(&mut h, "insert.table", json!({"rows": 2, "cols": 2}));
    assert_eq!(table_shape(&h), Some((2, 2, false)));
    menu(&mut h, &["Format", "Table"]);
    assert!(!is_greyed(&h, "Insert row above"), "enabled in a table");
    assert!(is_greyed(&h, "Merge cells"), "one cell: nothing to merge");
    entry(&h, "Insert row below").click();
    steps(&mut h, 3);
    assert_eq!(table_shape(&h), Some((3, 2, false)));
    menu(&mut h, &["Format", "Table", "Insert column right"]);
    assert_eq!(table_shape(&h), Some((3, 3, false)));
    menu(&mut h, &["Format", "Table", "Delete row"]);
    menu(&mut h, &["Format", "Table", "Delete column"]);
    assert_eq!(table_shape(&h), Some((2, 2, false)));
    menu(&mut h, &["Format", "Table", "Pin header row"]);
    assert_eq!(table_shape(&h), Some((2, 2, true)));
    menu(&mut h, &["Format", "Table", "Table properties"]);
    assert_eq!(h.state_mut().state()["dialogs"]["wordcraft"], true, "Table properties opens WordCraft's dialog");
    h.state_mut().app.dialog = None;
    steps(&mut h, 2);
    menu(&mut h, &["Format", "Table", "Delete table"]);
    assert_eq!(table_shape(&h), None);
}

#[test]
fn format_text_size_and_capitalisation() {
    let mut h = window(1400.0);
    type_text(&mut h, "hello world");
    run(&mut h, "select.all", json!({}));
    menu(&mut h, &["Format", "Text", "Capitalisation", "UPPERCASE"]);
    assert!(body_text(&mut h).contains("HELLO WORLD"), "{}", body_text(&mut h));
    menu(&mut h, &["Format", "Text", "Capitalisation", "Title Case"]);
    assert!(body_text(&mut h).contains("Hello World"), "{}", body_text(&mut h));
    menu(&mut h, &["Format", "Text", "Size", "Increase font size"]);
    assert_eq!(font_size(&mut h), json!(12.0));
    // Docs' keys: one point at a time.
    keys(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::Period);
    assert_eq!(font_size(&mut h), json!(13.0));
    keys(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::Comma);
    keys(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::Comma);
    assert_eq!(font_size(&mut h), json!(11.0));
}

#[test]
fn paragraph_styles_reach_heading_6_and_update_to_match() {
    let mut h = window(1400.0);
    let style =
        |h: &mut Harness<'static, Win>| h.state_mut().app.run("format.state", json!({})).unwrap()["styleName"].as_str().map(str::to_lowercase);
    menu(&mut h, &["Format", "Paragraph styles", "Heading 6", "Apply 'Heading 6'"]);
    assert_eq!(style(&mut h).as_deref(), Some("heading 6"));
    keys(&mut h, Modifiers::COMMAND | Modifiers::ALT, Key::Num4);
    assert_eq!(style(&mut h).as_deref(), Some("heading 4"));
    type_text(&mut h, "Title words");
    run(&mut h, "select.all", json!({}));
    run(&mut h, "format.underline", json!({}));
    menu(&mut h, &["Format", "Paragraph styles", "Heading 4", "Update 'Heading 4' to match"]);
    let doc = &h.state().app.session.doc;
    let h4 = doc.styles.find("Heading 4").unwrap();
    assert!(h4.chr.underline.is_some(), "Heading 4 now underlines: {:?}", h4.chr);
}

#[test]
fn line_and_paragraph_spacing_quick_items() {
    let mut h = window(1400.0);
    type_text(&mut h, "spacing");
    assert!(para(&h).space_before <= 0.0);
    menu(&mut h, &["Format", "Line and paragraph spacing", "Add space before paragraph"]);
    assert!(para(&h).space_before > 0.0);
    menu(&mut h, &["Format", "Line and paragraph spacing", "Remove space before paragraph"]);
    assert!(para(&h).space_before <= 0.0);
    let after = para(&h).space_after > 0.0;
    let label = if after { "Remove space after paragraph" } else { "Add space after paragraph" };
    menu(&mut h, &["Format", "Line and paragraph spacing", label]);
    assert_eq!(para(&h).space_after > 0.0, !after);
    assert!(!para(&h).keep_next && !para(&h).keep_lines);
    menu(&mut h, &["Format", "Line and paragraph spacing", "Keep with next"]);
    menu(&mut h, &["Format", "Line and paragraph spacing", "Keep lines together"]);
    assert!(para(&h).keep_next && para(&h).keep_lines);
}

#[test]
fn bullets_and_numbering_styles_and_restart_numbering() {
    let mut h = window(1400.0);
    type_text(&mut h, "one");
    menu(&mut h, &["Format", "Bullets and numbering", "List options"]);
    assert!(is_greyed(&h, "Restart numbering"), "not in a numbered list");
    close_menus(&mut h);
    menu(&mut h, &["Format", "Bullets and numbering", "Bullet style", "Star"]);
    assert!(para(&h).numbering.is_some_and(|n| n.num != 0), "a bulleted list");
    menu(&mut h, &["Format", "Bullets and numbering", "Numbering style", "i.  ii.  iii."]);
    let n = para(&h).numbering.unwrap();
    let lvl = h.state().app.session.doc.numbering.level(n.num, n.level).unwrap().clone();
    assert_eq!(lvl.format, wordcraft_doc::section::NumFormat::LowerRoman, "{lvl:?}");
    menu(&mut h, &["Format", "Bullets and numbering", "List options", "Restart numbering"]);
    assert_eq!(h.state_mut().state()["dialogs"]["restartNumbering"], true);
    keys(&mut h, Modifiers::NONE, Key::Backspace);
    type_text(&mut h, "5");
    click(&mut h, "OK");
    assert_eq!(h.state_mut().state()["dialogs"]["restartNumbering"], false);
    let n = para(&h).numbering.unwrap();
    let num = h.state().app.session.doc.numbering.nums.iter().find(|x| x.id == n.num).unwrap().clone();
    assert_eq!(num.start_overrides, vec![(0, 5)], "the list starts at 5");
}

fn comment_text(h: &mut Harness<'static, Win>, id: u32) -> String {
    let doc = &h.state().app.session.doc;
    doc.plain_text(wordcraft_doc::StoryRef::Part(doc.comments[&id].part))
}

#[test]
fn add_comment_takes_the_keyboard_and_writes_as_it_is_typed() {
    let mut h = window(1400.0);
    type_text(&mut h, "Some words here");
    h.state_mut().app.session.dirty = false;
    click(&mut h, "Add comment (Ctrl+Alt+M)");
    let id = *h.state().app.session.doc.comments.keys().next().expect("a comment was added");
    type_text(&mut h, "A note");
    assert_eq!(comment_text(&mut h, id), "A note", "typed into the new comment, written at once");
    assert_eq!(body_text(&mut h), "Some words here", "not into the page");
    assert!(h.state().app.session.dirty, "an unsaved change, so the page saves it");
}

#[test]
fn style_is_null_when_the_selection_mixes_paragraph_styles() {
    let mut h = window(1400.0);
    let app = &mut h.state_mut().app;
    app.run("text.insert", json!({"text": "Heading"})).unwrap();
    app.run("para.style", json!({"style": "Heading1"})).unwrap();
    app.run("text.newParagraph", json!({})).unwrap();
    app.run("text.insert", json!({"text": "Body"})).unwrap();
    app.run("para.style", json!({"style": "Normal"})).unwrap();
    app.run("select.all", json!({})).unwrap();
    let f = app.run("format.state", json!({})).unwrap();
    assert!(f["style"].is_null(), "{f}");
}

#[test]
fn style_box_is_empty_when_the_selection_mixes_paragraph_styles() {
    let mut h = window(1400.0);
    run(&mut h, "text.insert", json!({"text": "Heading"}));
    run(&mut h, "para.style", json!({"style": "Heading1"}));
    assert_eq!(h.state_mut().state()["toolbar"]["style"], "Heading 1");
    run(&mut h, "text.newParagraph", json!({}));
    run(&mut h, "text.insert", json!({"text": "Body"}));
    run(&mut h, "para.style", json!({"style": "Normal"}));
    assert_eq!(h.state_mut().state()["toolbar"]["style"], "Normal text");
    run(&mut h, "select.all", json!({}));
    assert_eq!(h.state_mut().state()["toolbar"]["style"], "", "Docs shows an empty style box");
}

#[test]
fn the_mini_toolbar_is_off_while_the_comments_panel_is_open() {
    let mut h = window(1400.0);
    let app = &mut h.state_mut().app;
    app.run("text.insert", json!({"text": "Some words"})).unwrap();
    app.run("select.all", json!({})).unwrap();
    assert!(wordcraft_ui_egui::mini_toolbar::visible(app), "a selection would show the bar");
    app.run("view.commentsPane", json!({"value": true})).unwrap();
    assert!(!wordcraft_ui_egui::mini_toolbar::visible(app), "not with the comments panel open");
}

/// A 4 × 2 red PNG.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAQAAAACCAIAAADwyuo0AAAAEklEQVR4nGP4z8AARwjWfwYGAG+qB/lC/d2tAAAAAElFTkSuQmCC";

/// The selected picture, and whether it floats (any wrapping but "In line with text").
fn selected_picture(h: &Harness<'static, Win>) -> Option<bool> {
    match wordcraft_engine::cmd::objects::selected(&h.state().app.session) {
        Some((_, o @ wordcraft_doc::para::InlineObject::Image { .. })) => Some(o.is_floating()),
        _ => None,
    }
}

#[test]
fn format_image_is_greyed_without_a_picture_and_wraps_a_selected_one() {
    let mut h = window(1400.0);
    let items = ["Crop image", "Size and rotation", "Alt text", "Replace image", "Reset image"];
    menu(&mut h, &["Format", "Image"]);
    for label in items {
        assert!(is_greyed(&h, label), "{label} is greyed with no picture selected");
    }
    close_menus(&mut h);
    run(&mut h, "insert.picture", json!({"data": PNG, "width": 100}));
    if selected_picture(&h).is_none() {
        run(&mut h, "select.all", json!({}));
    }
    assert_eq!(selected_picture(&h), Some(false), "an inline picture, selected");
    menu(&mut h, &["Format", "Image"]);
    for label in items {
        assert!(!is_greyed(&h, label), "{label} works on a selected picture");
    }
    entry(&h, "Text wrapping").click();
    steps(&mut h, 3);
    entry(&h, "Wrap text").click();
    steps(&mut h, 3);
    assert_eq!(selected_picture(&h), Some(true), "Wrap text floats it");
    menu(&mut h, &["Format", "Image", "Text wrapping", "In line with text"]);
    assert_eq!(selected_picture(&h), Some(false), "back in line");
    menu(&mut h, &["Format", "Image", "Alt text"]);
    assert!(h.state().app.session.view.alt_text_pane, "Alt text opens WordCraft's alt text panel");
    menu(&mut h, &["Format", "Image", "Size and rotation"]);
    assert_eq!(h.state_mut().state()["dialogs"]["wordcraft"], true, "Size and rotation opens WordCraft's size dialog");
}

#[test]
fn file_download_writes_other_formats_without_saving_the_document() {
    use std::{cell::RefCell, rc::Rc};
    let mut h = window(1400.0);
    let got: Rc<RefCell<Vec<(String, Vec<u8>)>>> = Rc::default();
    let keep = got.clone();
    h.state_mut().app.services.download = Some(Box::new(move |name: &str, bytes: &[u8]| {
        keep.borrow_mut().push((name.to_string(), bytes.to_vec()));
        Ok(())
    }));
    type_text(&mut h, "Downloaded words");
    assert!(h.state().app.session.dirty);
    for (label, ext) in [
        ("OpenDocument format (.odt)", "odt"),
        ("Rich Text Format (.rtf)", "rtf"),
        ("Plain text (.txt)", "txt"),
        ("Web page (.html)", "html"),
        ("Markdown (.md)", "md"),
    ] {
        menu(&mut h, &["File", "Download", label]);
        let (name, bytes) = got.borrow_mut().pop().unwrap_or_else(|| panic!("{label}: nothing downloaded"));
        assert!(name.ends_with(&format!(".{ext}")), "{label}: {name}");
        if ext == "odt" {
            assert!(bytes.starts_with(b"PK"), "an .odt is a zip package");
        } else {
            assert!(String::from_utf8_lossy(&bytes).contains("Downloaded words"), "{label} has the text");
        }
    }
    assert!(h.state().app.session.dirty, "a download is a copy: the document still needs saving");
    assert!(h.state().app.session.path.is_none(), "and it is still the same document");
}

/// The body's section breaks (paragraphs that end a section).
fn section_breaks(h: &Harness<'static, Win>) -> usize {
    let doc = &h.state().app.session.doc;
    doc.story(wordcraft_doc::StoryRef::Body).unwrap().iter().filter_map(|b| b.as_para()).filter(|p| p.section.is_some()).count()
}

#[test]
fn insert_break_headers_and_page_numbers() {
    let mut h = window(1400.0);
    type_text(&mut h, "One");
    menu(&mut h, &["Insert", "Break", "Section break (continuous)"]);
    assert_eq!(section_breaks(&h), 1);
    menu(&mut h, &["Insert", "Break", "Section break (next page)"]);
    assert_eq!(section_breaks(&h), 2);
    let before = body_text(&mut h).chars().count();
    menu(&mut h, &["Insert", "Break", "Column break"]);
    assert_eq!(body_text(&mut h).chars().count(), before + 1, "a column break character");

    let sect = |h: &Harness<'static, Win>| wordcraft_engine::cmd::page::sect(&h.state().app.session);
    assert!(!sect(&h).title_page);
    menu(&mut h, &["Insert", "Headers and footers", "Different first page"]);
    assert!(sect(&h).title_page);
    menu(&mut h, &["Insert", "Headers and footers", "Different odd and even"]);
    assert!(h.state().app.session.doc.settings.even_odd_headers);
    menu(&mut h, &["Insert", "Page numbers", "Number format", "i, ii, iii"]);
    assert_eq!(sect(&h).page_num_format, wordcraft_doc::section::NumFormat::LowerRoman);
    menu(&mut h, &["Insert", "Page numbers", "More options"]);
    assert_eq!(h.state_mut().state()["dialogs"]["wordcraft"], true, "WordCraft's page number dialog");
}

#[test]
fn tools_review_one_suggestion_at_a_time() {
    let mut h = window(1400.0);
    type_text(&mut h, "Kept ");
    run(&mut h, "review.trackChanges", json!({"value": true}));
    type_text(&mut h, "suggested");
    let changes = |h: &mut Harness<'static, Win>| h.state_mut().app.run("review.changes", json!({})).unwrap().as_array().map_or(0, Vec::len);
    assert_eq!(changes(&mut h), 1);
    menu(&mut h, &["Tools", "Review suggested edits", "Previous suggestion"]);
    menu(&mut h, &["Tools", "Review suggested edits", "Reject suggestion"]);
    assert_eq!(changes(&mut h), 0);
    assert_eq!(body_text(&mut h), "Kept ");
}

#[test]
fn table_of_contents_update_and_delete() {
    let mut h = window(1400.0);
    run(&mut h, "text.insert", json!({"text": "Chapter one"}));
    run(&mut h, "para.style", json!({"style": "Heading 1"}));
    run(&mut h, "text.newParagraph", json!({}));
    menu(&mut h, &["Insert", "Table of contents", "With page numbers"]);
    assert!(body_text(&mut h).matches("Chapter one").count() >= 2, "{}", body_text(&mut h));
    menu(&mut h, &["Insert", "Table of contents", "Update table of contents"]);
    menu(&mut h, &["Insert", "Table of contents", "Delete table of contents"]);
    assert_eq!(body_text(&mut h).matches("Chapter one").count(), 1, "{}", body_text(&mut h));
}

#[test]
fn tools_citations_style_and_source_dialog() {
    let mut h = window(1400.0);
    menu(&mut h, &["Tools", "Citations", "Citation style", "MLA"]);
    assert_eq!(h.state().app.session.bib_style, "MLA");
    menu(&mut h, &["Tools", "Citations", "Add citation source"]);
    assert_eq!(h.state_mut().state()["dialogs"]["wordcraft"], true, "WordCraft's source dialog");
}

#[test]
fn comment_cards_reply_resolve_and_delete_are_reachable() {
    let mut h = window(1400.0);
    type_text(&mut h, "Some words");
    run(&mut h, "select.all", json!({}));
    run(&mut h, "review.newComment", json!({"text": "Look here"}));
    run(&mut h, "view.commentsPane", json!({"value": true}));
    steps(&mut h, 3);
    for label in ["Reply", "Resolve", "Delete"] {
        assert!(h.query_all(By::new().label(label)).next().is_some(), "the comments panel shows {label}");
    }
}

#[test]
fn viewing_mode_refuses_edits_for_this_session_only() {
    let mut h = window(1400.0);
    type_text(&mut h, "Draft");
    click(&mut h, "Editing");
    click_containing(&mut h, "Read or print the final document");
    assert_eq!(h.state_mut().state()["mode"], "viewing");
    assert_eq!(h.state_mut().view("viewing"), true);
    assert_eq!(h.state_mut().view("showMarkup"), false, "Viewing shows the final document");
    type_text(&mut h, " more");
    assert_eq!(body_text(&mut h).trim_end(), "Draft", "typing is refused while viewing");
    assert!(h.state().app.session.doc.settings.protection.is_none(), "nothing is written into the document");
    assert!(h.get(By::new().label("Bold (Ctrl+B)")).accesskit_node().is_disabled(), "the toolbar is greyed");
    // The mode box and View › Mode still work.
    menu(&mut h, &["View", "Mode", "Editing: edit the document directly"]);
    assert_eq!(h.state_mut().state()["mode"], "editing");
    assert_eq!(h.state_mut().view("showMarkup"), true);
    type_text(&mut h, "!");
    assert_eq!(body_text(&mut h).trim_end(), "Draft!");
    menu(&mut h, &["View", "Mode", "Viewing: read or print the final document"]);
    assert_eq!(h.state_mut().state()["mode"], "viewing");
}

fn checklist_items(h: &Harness<'static, Win>) -> usize {
    let doc = &h.state().app.session.doc;
    let body = wordcraft_doc::StoryRef::Body;
    doc.para_paths(body).iter().filter(|p| wordcraft_engine::cmd::checklist::item_box(doc, body, p).is_some()).count()
}

#[test]
fn checklist_from_the_menu_and_ctrl_shift_9() {
    let mut h = window(1400.0);
    type_text(&mut h, "Milk");
    keys(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::Num9);
    assert_eq!(checklist_items(&h), 1, "Ctrl+Shift+9 makes a checklist");
    keys(&mut h, Modifiers::NONE, Key::Enter);
    type_text(&mut h, "Eggs");
    assert_eq!(checklist_items(&h), 2, "Enter starts the next item");
    run(&mut h, "select.all", json!({}));
    menu(&mut h, &["Format", "Bullets and numbering", "Checklist"]);
    assert_eq!(checklist_items(&h), 0, "Checklist again takes the boxes away");
    assert_eq!(body_text(&mut h).trim_end().replace('\r', "\n"), "Milk\nEggs");
}

#[test]
fn a_pictures_right_click_menu_offers_format_image() {
    let mut h = window(1400.0);
    run(&mut h, "insert.picture", json!({"data": PNG, "width": 100}));
    if selected_picture(&h).is_none() {
        run(&mut h, "select.all", json!({}));
    }
    assert!(selected_picture(&h).is_some());
    // Shift+F10 opens the page's right-click menu, as a right-click on the picture does.
    h.state_mut().app.canvas.context_menu_from_keys = true;
    steps(&mut h, 3);
    let labels: Vec<String> =
        h.query_all(By::new().role(Role::Button)).map(|n| n.accesskit_node().label().unwrap_or_default().trim().to_string()).collect();
    assert!(!labels.iter().any(|l| l.starts_with("Size and Crop")), "not WordCraft's item, which opens a hidden ribbon tab: {labels:?}");
    for label in ["Crop image", "Size and rotation", "Text wrapping", "Alt text", "Replace image", "Reset image"] {
        assert!(labels.iter().any(|l| l == label), "Format › Image's {label} is in the menu: {labels:?}");
    }
    click_containing(&mut h, "Size and rotation");
    assert_eq!(h.state_mut().state()["dialogs"]["wordcraft"], true, "Size and rotation opens WordCraft's size dialog");
}
