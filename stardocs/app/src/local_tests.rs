//! The desktop program's local mode, tested natively: our window over a file on disk, driven
//! through egui_kittest as shell_tests.rs drives the web window, with a fake clock for the quiet
//! moment before a save.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "tests")]

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use egui::Event;
use egui_kittest::Harness;
use serde_json::json;
use wordcraft_ui_egui::{Services, WordApp};

use crate::localfile::{Folders, Link, LocalFile, Saved, temp_dir};

struct Win {
    app: WordApp,
    shell: crate::shell::Shell,
    link: Link,
    /// The window title last sent (native.rs's `keep_title`).
    title: String,
    /// Each frame's window titles, in the order sent, for frames that sent any.
    titles: Vec<Vec<String>>,
}

/// A clock the test moves on by hand (milliseconds).
type Clock = Rc<Cell<f64>>;

fn folders(dir: &Path) -> Folders {
    Folders { documents: dir.join("home/Documents/Starbase"), data: dir.join("home/data") }
}

/// The window on `file` (or a new document), as native.rs sets it up.
fn window(dir: &Path, file: Option<PathBuf>, clock: &Clock) -> Harness<'static, Win> {
    window_sized(dir, file, clock, 1400.0)
}

/// [`window`], `width` points wide.
fn window_sized(dir: &Path, file: Option<PathBuf>, clock: &Clock, width: f32) -> Harness<'static, Win> {
    let store: Box<dyn crate::localfile::Store> = match file.clone() {
        Some(f) => Box::new(LocalFile::open(f)),
        None => Box::new(LocalFile::new_in(folders(dir).documents)),
    };
    let c = clock.clone();
    let link = Link::new(store, folders(dir)).with_clock(move || c.get());
    let mut app = crate::native::make_app(&link, Services::default(), None);
    app.ui.language = "en".into();
    if file.is_some() {
        assert!(link.open(&mut app), "{:?}", link.notice());
    }
    let win = Win { app, shell: crate::shell::Shell::default(), link, title: String::new(), titles: Vec::new() };
    let mut h = Harness::builder().with_size(egui::vec2(width, 900.0)).build_ui_state(
        |ui, w: &mut Win| {
            egui_extras::install_image_loaders(ui.ctx());
            w.app.logic(ui.ctx());
            w.link.tick(&mut w.app, ui.ctx());
            w.shell.ui(&mut w.app, ui, Some(&w.link));
            w.app.ui(ui);
            crate::native::keep_title(ui.ctx(), &crate::native::title(Some(&w.link.name())), &mut w.title);
        },
        win,
    );
    steps(&mut h, 6);
    h
}

fn steps(h: &mut Harness<'static, Win>, n: usize) {
    for _ in 0..n {
        let mut raw = std::mem::take(h.input_mut());
        let ctx = h.ctx.clone();
        h.state_mut().app.raw_input_hook(&ctx, &mut raw);
        *h.input_mut() = raw;
        h.step();
        let titles: Vec<String> = h
            .output()
            .viewport_output
            .values()
            .flat_map(|v| &v.commands)
            .filter_map(|c| match c {
                egui::ViewportCommand::Title(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        if !titles.is_empty() {
            h.state_mut().titles.push(titles);
        }
    }
}

fn type_text(h: &mut Harness<'static, Win>, text: &str) {
    h.input_mut().events.push(Event::Text(text.into()));
    steps(h, 3);
}

/// Lets the quiet moment pass, so the window saves by itself.
fn wait_for_save(h: &mut Harness<'static, Win>, clock: &Clock) {
    clock.set(clock.get() + 3000.0);
    steps(h, 3);
}

fn body_text(h: &Harness<'static, Win>) -> String {
    h.state().app.session.doc.plain_text(wordcraft_doc::StoryRef::Body)
}

/// A Word file holding `text`, as another program would leave it.
fn fixture(path: &Path, text: &str) {
    fixture_zoomed(path, text, None);
}

/// [`fixture`], saved at `zoom` per cent (Word's `w:zoom`).
fn fixture_zoomed(path: &Path, text: &str, zoom: Option<u32>) {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::blank_document(&Default::default())), Services::default());
    app.run("text.insert", json!({ "text": text })).unwrap();
    app.session.doc.settings.zoom = zoom.map(|percent| wordcraft_doc::Zoom { percent, fit: None });
    let bytes = wordcraft_engine::io::save_bytes_with("fixture.docx", &app.session.doc, None, &Default::default()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

#[test]
fn typing_saves_back_to_the_file_and_reopening_shows_it() {
    let dir = temp_dir("roundtrip");
    let path = dir.join("Letter.docx");
    fixture(&path, "Fixture text");
    let clock = Clock::default();
    let mut h = window(&dir, Some(path.clone()), &clock);
    assert!(body_text(&h).contains("Fixture text"));
    assert_eq!(h.state().link.name(), "Letter.docx");
    type_text(&mut h, "Typed here ");
    assert!(h.state().app.session.dirty);
    let before = std::fs::read(&path).unwrap();
    wait_for_save(&mut h, &clock);
    assert_eq!(h.state().link.saved(), Saved::Done);
    assert!(!h.state().app.session.dirty);
    assert_ne!(std::fs::read(&path).unwrap(), before, "the file was written");
    // Nothing else beside it: no temporary file, no copy.
    assert_eq!(files_in(&dir), ["Letter.docx", "home"]);
    let again = window(&dir, Some(path), &clock);
    let text = body_text(&again);
    assert!(text.contains("Typed here") && text.contains("Fixture text"), "{text:?}");
    // The file is in the recent list.
    assert_eq!(crate::localfile::recent(&folders(&dir)).first(), Some(&dir.join("Letter.docx")));
}

#[test]
fn a_file_changed_by_another_program_is_kept_and_ours_saved_as_a_copy() {
    let dir = temp_dir("conflict");
    let path = dir.join("Report.docx");
    fixture(&path, "Original");
    let clock = Clock::default();
    let mut h = window(&dir, Some(path.clone()), &clock);
    type_text(&mut h, "Mine ");
    // Another program saves the file meanwhile.
    fixture(&path, "Someone else's much longer version of the report");
    let theirs = std::fs::read(&path).unwrap();
    wait_for_save(&mut h, &clock);
    assert_eq!(std::fs::read(&path).unwrap(), theirs, "never overwritten");
    let name = h.state().link.name();
    assert_eq!(name, "Report (conflicted copy).docx");
    assert!(h.state().link.notice().is_some_and(|n| n.contains("changed elsewhere")));
    assert_eq!(files_in(&dir).len(), 3, "the file, our copy, and home");
    let copy = window(&dir, Some(dir.join(&name)), &clock);
    assert!(body_text(&copy).contains("Mine"));
    // Later saves go on to the copy.
    type_text(&mut h, "More ");
    wait_for_save(&mut h, &clock);
    assert_eq!(h.state().link.name(), name);
    assert_eq!(std::fs::read(&path).unwrap(), theirs);
}

#[test]
fn a_new_document_is_made_in_documents_starbase_on_its_first_save() {
    let dir = temp_dir("newdoc");
    let clock = Clock::default();
    let mut h = window(&dir, None, &clock);
    assert_eq!(h.state().link.name(), "");
    type_text(&mut h, "Shopping list");
    wait_for_save(&mut h, &clock);
    assert_eq!(h.state().link.saved(), Saved::Done);
    let docs = folders(&dir).documents;
    assert_eq!(files_in(&docs), ["Shopping list.docx"]);
    assert_eq!(crate::native::title(Some(&h.state().link.name())), "Shopping list - Starbase Docs");
    assert_eq!(crate::localfile::recent(&folders(&dir)), [docs.join("Shopping list.docx")]);
}

/// File › Make a copy on this computer: what isn't saved yet is saved first, then "Copy of
/// <name>" is made beside the file and opened in a window of its own.
#[test]
fn make_a_copy_copies_the_file_beside_it_and_opens_the_copy() {
    use crate::localfile::Request;
    let dir = temp_dir("makecopy");
    let path = dir.join("Letter.docx");
    fixture(&path, "Dear Sam");
    let clock = Clock::default();
    let mut h = window(&dir, Some(path.clone()), &clock);
    type_text(&mut h, "Unsaved ");
    {
        let Win { app, shell, link, .. } = h.state_mut();
        assert_eq!(shell.test_command(app, Some(link), "stardocs.makeCopy", json!({})), Ok(json!(true)));
    }
    steps(&mut h, 2);
    let copy = dir.join("Copy of Letter.docx");
    assert_eq!(h.state().link.take_requests(), [Request::Open(copy.clone())]);
    assert!(!h.state().app.session.dirty, "saved first");
    assert_eq!(std::fs::read(&copy).unwrap(), std::fs::read(&path).unwrap(), "the copy is the saved file");
    let again = window(&dir, Some(copy), &clock);
    assert!(body_text(&again).contains("Unsaved"), "with what was typed");
    assert_eq!(h.state().link.name(), "Letter.docx", "this window keeps the document itself");
    // A second copy doesn't overwrite the first.
    let Win { app, shell, link, .. } = h.state_mut();
    shell.test_command(app, Some(link), "stardocs.makeCopy", json!({})).unwrap();
    assert_eq!(link.take_requests(), [Request::Open(dir.join("Copy of Letter (2).docx"))]);
}

#[test]
fn there_is_no_share_button_for_a_local_document() {
    use egui_kittest::kittest::Queryable;
    let dir = temp_dir("share");
    let clock = Clock::default();
    let h = window(&dir, None, &clock);
    assert!(h.query_by_label("Share").is_none());
}

/// WordCraft names the window "<name> - WordCraft" whenever its title changes (opening, the
/// first change after a save); the window keeps ours, sent after it in the same frame.
#[test]
fn the_window_title_is_ours_not_wordcrafts() {
    let dir = temp_dir("title");
    let path = dir.join("letter.docx");
    fixture(&path, "Dear Sam");
    let clock = Clock::default();
    let mut h = window(&dir, Some(path), &clock);
    type_text(&mut h, "Hello ");
    wait_for_save(&mut h, &clock);
    type_text(&mut h, "again ");
    let titles = &h.state().titles;
    assert!(titles.iter().flatten().any(|t| t.ends_with("- WordCraft")), "WordCraft sent no title of its own: {titles:?}");
    for frame in titles {
        assert_eq!(frame.last().map(String::as_str), Some("letter - Starbase Docs"), "{titles:?}");
    }
}

/// A file saved at 146 % (python-docx's template) opens at 100 % when the page fits the window,
/// as Google Docs opens every document, and at the page's width when it doesn't.
#[test]
fn a_document_opens_at_100_percent_or_the_page_width() {
    let dir = temp_dir("zoom");
    let path = dir.join("zoomed.docx");
    fixture_zoomed(&path, "Big", Some(146));
    let clock = Clock::default();
    let h = window_sized(&dir, Some(path.clone()), &clock, 1400.0);
    let v = &h.state().app.session.view;
    assert_eq!((v.zoom, v.fit.as_str()), (1.0, ""));
    // A US Letter page is 816 px wide at 100 %: it doesn't fit 700.
    let narrow = window_sized(&dir, Some(path), &clock, 700.0);
    assert_eq!(narrow.state().app.session.view.fit, "pageWidth");
    // A zoom the user picks stays.
    let mut h = h;
    h.state_mut().app.run("view.zoom", json!({ "value": 150 })).unwrap();
    steps(&mut h, 3);
    assert_eq!(h.state().app.session.view.zoom, 1.5);
}

/// Calibri and Cambria are drawn in the bundled Carlito and Caladea (their widths) on a computer
/// without them, not in a fallback serif.
#[test]
fn calibri_and_cambria_resolve_to_carlito_and_caladea() {
    crate::bundled_fonts::add();
    let db = wordcraft_fonts::FontDb::global();
    for (font, twin) in [("Calibri", "Carlito"), ("Cambria", "Caladea")] {
        assert!(db.has_family(twin), "{twin} isn't loaded");
        if db.has_family(font) {
            continue; // the real font is installed here: it is used, as it should be
        }
        for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
            let r = wordcraft_fonts::word::resolve(font, bold, italic);
            assert_eq!(r.face.get().family, twin, "{font} bold={bold} italic={italic}");
            assert!(!r.synth_bold && !r.synth_italic, "{font}: {twin} has its own bold and italic faces");
        }
    }
}

#[test]
fn the_command_line_opens_local_files_or_a_server_document() {
    use crate::native::{Args, parse_args};
    let os = |v: &[&str]| v.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
    let a = parse_args(os(&["--server-file", "abc123", "--name", "Report.docx"])).unwrap();
    assert_eq!(a.server_file, Some(("abc123".to_string(), "Report.docx".to_string())));
    assert!(a.files.is_empty());
    let a = parse_args(os(&["--name", "N.docx", "--server-file", "x"])).unwrap();
    assert_eq!(a.server_file, Some(("x".to_string(), "N.docx".to_string())), "in either order");
    let a = parse_args(os(&["/tmp/a.docx", "/tmp/b.odt"])).unwrap();
    assert_eq!(a.files, vec![PathBuf::from("/tmp/a.docx"), PathBuf::from("/tmp/b.odt")]);
    assert_eq!(parse_args(os(&["--version"])).unwrap(), Args { version: true, ..Default::default() });
    assert!(parse_args(os(&["--server-file"])).is_err(), "needs an id");
    assert!(parse_args(os(&["--server-file", ""])).is_err());
    assert!(parse_args(os(&["--bogus"])).is_err());
}

#[test]
fn a_first_start_sizes_the_window_to_80_percent_of_the_screen() {
    use crate::native::first_size;
    assert_eq!(first_size(egui::vec2(1920.0, 1080.0)), egui::vec2(1536.0, 864.0));
    // A small laptop screen: 80 %, never under the smallest size, never over the screen.
    assert!((first_size(egui::vec2(1366.0, 768.0)) - egui::vec2(1092.8, 614.4)).length() < 0.01);
    assert_eq!(first_size(egui::vec2(700.0, 500.0)), egui::vec2(640.0, 480.0));
    assert_eq!(first_size(egui::vec2(600.0, 400.0)), egui::vec2(600.0, 400.0));
}
