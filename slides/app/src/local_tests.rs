//! The desktop program's local files, tested natively: our window over a .pptx on disk, driven
//! through egui_kittest as shell_tests.rs drives the web window, with a fake clock for the quiet
//! moment before a save.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "tests")]

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use deckcraft_ui_egui::SlideApp;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::localfile::{Folders, Link, LocalFile, NEW_NAME, temp_dir};
use crate::shell::{Drive, Saved};

struct Win {
    app: SlideApp,
    shell: crate::shell::Shell,
    link: Link,
}

type Clock = Rc<Cell<f64>>;

fn folders(dir: &Path) -> Folders {
    Folders { documents: dir.join("home/Documents/Starbase"), data: dir.join("home/data") }
}

/// The window on `file` (or a new presentation), as native.rs sets it up.
fn window(dir: &Path, file: Option<PathBuf>, clock: &Clock) -> Harness<'static, Win> {
    let store: Box<dyn crate::localfile::Store> = match file.clone() {
        Some(f) => Box::new(LocalFile::open(f)),
        None => Box::new(LocalFile::new_in(folders(dir).documents)),
    };
    let c = clock.clone();
    let link = Link::new(store, folders(dir)).with_clock(move || c.get());
    let mut app = crate::native::make_app(&link, deckcraft_ui_egui::Services::default(), file.is_none());
    if file.is_some() {
        assert!(link.open(&mut app), "{:?}", link.notice());
    }
    let win = Win { app, shell: crate::shell::Shell::default(), link };
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
        |ui, w: &mut Win| {
            w.app.logic(ui.ctx());
            w.link.tick(&mut w.app, ui.ctx());
            w.shell.ui(&mut w.app, ui, Some(&w.link));
            w.app.ui(ui);
        },
        win,
    );
    egui_extras::install_image_loaders(&h.ctx);
    h.run_steps(3);
    h
}

fn slides(app: &SlideApp) -> usize {
    app.session.active().map_or(0, |d| d.doc.slides.len())
}

/// Lets the quiet moment pass, so the window saves by itself.
fn wait_and_save(h: &mut Harness<'static, Win>, clock: &Clock) {
    h.run_steps(2);
    clock.set(clock.get() + 3000.0);
    h.run_steps(3);
}

/// A .pptx with `n` slides, written to `path`.
fn deck(path: &Path, n: usize) {
    let mut app = SlideApp::new(deckcraft_engine::Session::new(), deckcraft_ui_egui::Services::default());
    app.run("file.new", json!({})).unwrap();
    for _ in 1..n {
        app.run("slide.new", json!({})).unwrap();
    }
    let doc = app.session.active().unwrap().doc.clone();
    std::fs::write(path, deckcraft_engine::cmd::file::save_bytes(&doc, "pptx").unwrap()).unwrap();
}

#[test]
fn a_new_presentation_is_a_named_pptx_in_documents_starbase_on_its_first_change() {
    let dir = temp_dir("new");
    let clock: Clock = Rc::default();
    let mut h = window(&dir, None, &clock);
    assert!(h.query_by_label("Untitled presentation").is_some(), "the title row");
    let docs = folders(&dir).documents;
    assert!(!docs.join(NEW_NAME).exists(), "nothing is written before a change");
    h.state_mut().app.run("slide.new", json!({})).unwrap();
    wait_and_save(&mut h, &clock);
    let saved = docs.join(NEW_NAME);
    let bytes = std::fs::read(&saved).unwrap_or_default();
    assert!(bytes.starts_with(b"PK"), "a PowerPoint (zip) file at {}", saved.display());
    assert_eq!(h.state().link.saved(), Saved::Done);
    assert!(h.query_by_label("Saved on this computer").is_some());
    assert_eq!(h.state().link.recent(), Vec::<PathBuf>::new(), "the open file isn't in its own recent list");
    let names: Vec<_> = std::fs::read_dir(&docs).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, [NEW_NAME], "one file, no .deckcraft, no temporary file");
}

#[test]
fn an_existing_file_opens_and_saves_back_to_itself() {
    let dir = temp_dir("open");
    let path = dir.join("Plan.pptx");
    deck(&path, 2);
    let clock: Clock = Rc::default();
    let mut h = window(&dir, Some(path.clone()), &clock);
    assert_eq!(slides(&h.state().app), 2);
    assert!(h.query_by_label("Plan").is_some(), "the name without .pptx");
    h.state_mut().app.run("slide.new", json!({})).unwrap();
    wait_and_save(&mut h, &clock);
    let again = window(&dir, Some(path.clone()), &clock);
    assert_eq!(slides(&again.state().app), 3, "the saved file has the new slide");
    assert_eq!(crate::localfile::recent(&folders(&dir)).first(), Some(&path), "on the recent list");
}

#[test]
fn a_file_changed_elsewhere_is_kept_and_ours_becomes_a_conflicted_copy() {
    let dir = temp_dir("conflict");
    let path = dir.join("Talk.pptx");
    deck(&path, 1);
    let clock: Clock = Rc::default();
    let mut h = window(&dir, Some(path.clone()), &clock);
    deck(&path, 4); // someone else saves a bigger one
    let theirs = std::fs::read(&path).unwrap();
    h.state_mut().app.run("slide.new", json!({})).unwrap();
    wait_and_save(&mut h, &clock);
    assert_eq!(std::fs::read(&path).unwrap(), theirs, "theirs is untouched");
    let copy = dir.join("Talk (conflicted copy).pptx");
    assert!(std::fs::read(&copy).unwrap_or_default().starts_with(b"PK"), "ours is kept beside it");
    assert_eq!(h.state().link.name(), "Talk (conflicted copy).pptx", "and is open now");
}

#[test]
fn a_deckcraft_file_is_never_saved() {
    let dir = temp_dir("deckcraft");
    let clock: Clock = Rc::default();
    let mut h = window(&dir, None, &clock);
    let save = h.state_mut().app.services.host_save.as_mut().unwrap();
    assert!(save("Talk.deckcraft", b"x", true).is_err(), "Starbase files stay .pptx");
    assert!(!folders(&dir).documents.join("Talk.deckcraft").exists());
}

#[test]
fn the_file_menu_has_open_recent_and_connect() {
    let dir = temp_dir("menu");
    let clock: Clock = Rc::default();
    let mut h = window(&dir, None, &clock);
    h.get_by_label("File").click();
    h.run_steps(3);
    for item in ["New presentation", "Open…", "Open recent", "Connect to a server…"] {
        // A row with a shortcut is named with it ("Open… Ctrl+O").
        assert!(h.query_by_label_contains(item).is_some(), "File › {item}");
    }
    assert!(h.query_by_label("Open from Starbase").is_none(), "the browser's wording is not here");
    let s = h.state_mut();
    s.shell.command(&mut s.app, Some(&s.link as &dyn Drive), "slides.connect", json!({})).unwrap();
    s.shell.command(&mut s.app, Some(&s.link as &dyn Drive), "slides.new", json!({})).unwrap();
    use crate::localfile::Request;
    assert_eq!(s.link.take_requests(), [Request::Connect, Request::New]);
}
