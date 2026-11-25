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
}

/// A clock the test moves on by hand (milliseconds).
type Clock = Rc<Cell<f64>>;

fn folders(dir: &Path) -> Folders {
    Folders { documents: dir.join("home/Documents/Starbase"), data: dir.join("home/data") }
}

/// The window on `file` (or a new document), as native.rs sets it up.
fn window(dir: &Path, file: Option<PathBuf>, clock: &Clock) -> Harness<'static, Win> {
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
    let win = Win { app, shell: crate::shell::Shell::default(), link };
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
        |ui, w: &mut Win| {
            egui_extras::install_image_loaders(ui.ctx());
            w.app.logic(ui.ctx());
            w.link.tick(&mut w.app, ui.ctx());
            w.shell.ui(&mut w.app, ui, Some(&w.link));
            w.app.ui(ui);
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
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::blank_document(&Default::default())), Services::default());
    app.run("text.insert", json!({ "text": text })).unwrap();
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
    assert!(name.starts_with("Report (Starbase Docs copy ") && name.ends_with(".docx"), "{name}");
    assert!(h.state().link.notice().is_some_and(|n| n.contains("changed by another program")));
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

#[test]
fn share_is_greyed_for_a_local_document() {
    use egui_kittest::kittest::{NodeT, Queryable};
    let dir = temp_dir("share");
    let clock = Clock::default();
    let h = window(&dir, None, &clock);
    let share = h.get_by_label("Share");
    assert!(share.accesskit_node().is_disabled());
}
