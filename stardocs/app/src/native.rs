//! The desktop program: the same window as the web app (shell.rs over WordCraft's page) in a
//! native eframe window, drawn with OpenGL through glow as the web build draws with WebGL2.
//!
//! Usage: `stardocs [FILE]`. Local mode only for now: the document is a file on this computer
//! (localfile.rs), saved back to itself a moment after each change. With no file, a new
//! document is made in `~/Documents/Starbase` on its first save. Connecting to a Starbase server
//! is a later job: another `localfile::Store` behind the same `Link`.

use std::path::{Path, PathBuf};

use wordcraft_engine::Session;
use wordcraft_ui_egui::{FileDialogAnswer, FileDialogRequest, HostChrome, Services, WordApp};

use crate::localfile::{self, Folders, Link, LocalFile, Request};

/// The desktop entry's app id (packaging/linux), also the window's Wayland app id / X11 class.
pub const APP_ID: &str = "io.github.DomeShifter.Starbase.Docs";

const USAGE: &str = "\
stardocs: Starbase Docs, a word processor

USAGE:
  stardocs [FILE]

  FILE                a document to open (.docx, .odt, .rtf, .txt…); without one, a new
                      document is made in Documents/Starbase when you first type
  --version           print the version
  -h, --help          print this help
";

/// Parses the command line and runs the window until it closes.
pub fn main() -> std::process::ExitCode {
    env_logger_lite();
    let mut file: Option<PathBuf> = None;
    for a in std::env::args_os().skip(1) {
        match a.to_str() {
            Some("--version") => {
                let _ = std::io::Write::write_fmt(&mut std::io::stdout(), format_args!("stardocs {}\n", env!("CARGO_PKG_VERSION")));
                return std::process::ExitCode::SUCCESS;
            }
            Some("-h" | "--help") => {
                let _ = std::io::Write::write_all(&mut std::io::stdout(), USAGE.as_bytes());
                return std::process::ExitCode::SUCCESS;
            }
            Some(s) if s.starts_with("--") => {
                let _ = std::io::Write::write_fmt(&mut std::io::stderr(), format_args!("stardocs: unknown option {s}\n\n{USAGE}"));
                return std::process::ExitCode::from(2);
            }
            _ if file.is_none() => file = Some(std::path::absolute(PathBuf::from(&a)).unwrap_or_else(|_| PathBuf::from(&a))),
            _ => {
                // One document per window: further files open in windows of their own.
                spawn(Some(Path::new(&a)));
            }
        }
    }
    let folders = Folders::of_user().unwrap_or_else(|| Folders { documents: PathBuf::from("Starbase"), data: PathBuf::from(".starbase") });
    if let Err(e) = std::fs::create_dir_all(&folders.documents) {
        log::warn!("{}: {e}", folders.documents.display());
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(title(file.as_ref().and_then(|f| f.file_name()).map(|n| n.to_string_lossy().into_owned()).as_deref()))
        .with_app_id(APP_ID)
        .with_inner_size([1280.0, 900.0])
        .with_min_inner_size([640.0, 480.0])
        .with_drag_and_drop(true);
    if let Some(icon) = icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Glow, ..Default::default() };
    let run = eframe::run_native(
        "Starbase Docs",
        options,
        Box::new(move |cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(Desktop::new(&cc.egui_ctx, file, folders)))
        }),
    );
    match run {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            let _ = std::io::Write::write_fmt(&mut std::io::stderr(), format_args!("stardocs: {e}\n"));
            std::process::ExitCode::FAILURE
        }
    }
}

/// Warnings and errors to standard error (eframe and WordCraft log through `log`).
fn env_logger_lite() {
    struct Stderr;
    impl log::Log for Stderr {
        fn enabled(&self, m: &log::Metadata) -> bool {
            m.level() <= log::Level::Warn
        }
        fn log(&self, r: &log::Record) {
            if self.enabled(r.metadata()) {
                let _ = std::io::Write::write_fmt(&mut std::io::stderr(), format_args!("stardocs: {}: {}\n", r.level(), r.args()));
            }
        }
        fn flush(&self) {}
    }
    static LOGGER: Stderr = Stderr;
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Warn);
    }
}

/// "<name> - Starbase Docs", as the web page's tab.
pub fn title(name: Option<&str>) -> String {
    let stem = match name.filter(|n| !n.is_empty()) {
        Some(n) => n.rsplit_once('.').map_or(n, |(s, _)| s),
        None => "Untitled document",
    };
    format!("{stem} - Starbase Docs")
}

/// The window icon: app/icons/stardocs_logo.svg drawn at 256 px.
fn icon() -> Option<egui::IconData> {
    let svg: &[u8] = include_bytes!("../icons/stardocs_logo.svg");
    let img = egui_extras::image::load_svg_bytes_with_size(
        svg,
        egui::SizeHint::Size { width: 256, height: 256, maintain_aspect_ratio: true },
        &Default::default(),
    )
    .map_err(|e| log::warn!("window icon: {e}"))
    .ok()?;
    Some(egui::IconData { rgba: img.as_raw().to_vec(), width: img.width() as u32, height: img.height() as u32 })
}

/// Starts another window of this program, on `file` or on a new document.
fn spawn(file: Option<&Path>) {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            log::error!("can't find this program to open another window: {e}");
            return;
        }
    };
    let mut cmd = std::process::Command::new(exe);
    if let Some(f) = file {
        cmd.arg(f);
    }
    if let Err(e) = cmd.spawn() {
        log::error!("can't open another window: {e}");
    }
}

/// Where the editor's preferences (theme, interface size, editing options…) are kept, as the
/// web app keeps them in `localStorage`.
fn prefs_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from)?;
    let config = std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()).map_or_else(|| home.join(".config"), PathBuf::from);
    Some(config.join("starbase").join("docs-ui.json"))
}

/// WordCraft's own file dialogs (Insert › Image, Add font…): the system's, on a thread of their
/// own so the window keeps drawing.
fn file_dialog(req: FileDialogRequest) -> std::sync::mpsc::Receiver<FileDialogAnswer> {
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("file dialog".into()).spawn(move || {
        let d = rfd::AsyncFileDialog::new();
        let picked: Vec<rfd::FileHandle> = match req {
            FileDialogRequest::Open { purpose } => pollster::block_on(filters(d, &purpose).pick_file()).into_iter().collect(),
            FileDialogRequest::OpenMany { purpose } => pollster::block_on(filters(d, &purpose).pick_files()).unwrap_or_default(),
            FileDialogRequest::Save { name, dir } => {
                let mut d = d.set_file_name(name);
                if let Some(dir) = dir.filter(|d| Path::new(d).is_dir()) {
                    d = d.set_directory(dir);
                }
                pollster::block_on(d.save_file()).into_iter().collect()
            }
        };
        let _ = tx.send(Ok(picked.iter().map(|f| f.path().to_string_lossy().into_owned()).collect()));
    });
    if let Err(e) = spawned {
        log::error!("file dialog: {e}");
    }
    rx
}

fn filters(d: rfd::AsyncFileDialog, purpose: &str) -> rfd::AsyncFileDialog {
    match purpose {
        "picture" => d.add_filter("Pictures", &["png", "jpg", "jpeg", "gif", "webp", "bmp", "wmf", "emf"]),
        "font" => d.add_filter("Fonts", wordcraft_fonts::FONT_FILE_EXTS),
        "recipients" => d.add_filter("Recipient lists", wordcraft_engine::cmd::mailings::RECIPIENT_EXTENSIONS),
        _ => d.add_filter("Documents", localfile::DOC_EXTS),
    }
}

/// A blank document set up as web.rs sets it up (Docs' look, the editor's preferences), with
/// the link's saves in place of WordCraft's own.
pub fn make_app(link: &Link, services: Services, stored_prefs: Option<&str>) -> WordApp {
    let region = wordcraft_ui_egui::i18n::system_region();
    let mut doc = wordcraft_engine::blank_document(&region);
    crate::shell::docs_defaults(&mut doc);
    let mut session = Session::new(doc);
    session.system_region = region;
    let mut services = services;
    link.connect(&mut services);
    let mut app = WordApp::new(session, services).with_chrome(HostChrome { title_bar: false, ribbon: false, status_bar: false });
    // The link saves (a moment after the last change); WordCraft's own AutoSave stays off.
    app.autosave = false;
    crate::prefs::restore(&mut app, stored_prefs);
    crate::theme::keep_light(&mut app);
    crate::looks::apply(&mut app);
    if app.session.prefs.region.is_some() {
        let mut doc = app.session.blank_document();
        crate::shell::docs_defaults(&mut doc);
        app.replace_startup_document(doc);
    }
    app
}

/// Whether the window holds nothing yet: a new document no one has typed in.
fn is_blank(app: &WordApp, link: &Link) -> bool {
    link.location().is_none() && !app.session.dirty && app.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim().is_empty()
}

struct Desktop {
    app: WordApp,
    shell: crate::shell::Shell,
    link: Link,
    folders: Folders,
    prefs: crate::prefs::PrefsSync,
    /// The window title last set.
    title: String,
    /// A close was refused once because the document couldn't be saved: the next one quits.
    close_refused: bool,
}

impl Desktop {
    fn new(ctx: &egui::Context, file: Option<PathBuf>, folders: Folders) -> Self {
        crate::arimo::add();
        let stored = prefs_path().and_then(|p| std::fs::read_to_string(p).ok());
        let link = match &file {
            Some(f) => Link::new(Box::new(LocalFile::open(f.clone())), folders.clone()),
            None => Link::new(Box::new(LocalFile::new_in(folders.documents.clone())), folders.clone()),
        };
        let services = Services { file_dialog: Some(Box::new(file_dialog)), ..Default::default() };
        let mut app = make_app(&link, services, stored.as_deref());
        if file.is_some() {
            link.open(&mut app);
        }
        ctx.request_repaint();
        Self {
            app,
            shell: crate::shell::Shell::default(),
            link,
            folders,
            prefs: crate::prefs::PrefsSync::new(stored),
            title: String::new(),
            close_refused: false,
        }
    }

    /// Opens `path` here when this window holds nothing yet, else in a window of its own.
    fn open(&mut self, path: PathBuf) {
        if self.link.location().as_deref() == Some(path.as_path()) {
            return;
        }
        if !is_blank(&self.app, &self.link) {
            spawn(Some(&path));
            return;
        }
        let link = Link::new(Box::new(LocalFile::open(path)), self.folders.clone());
        link.connect(&mut self.app.services);
        if link.open(&mut self.app) {
            self.link = link;
        } else {
            // Keep the reason it couldn't open: it is in the new link's notice.
            self.link.connect(&mut self.app.services);
            if let Some(why) = link.notice() {
                self.app.status(why);
            }
        }
    }

    fn save_prefs(&mut self) {
        let Some(json) = self.prefs.changed(&self.app) else { return };
        let Some(path) = prefs_path() else { return };
        let written = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| localfile::write_atomic(&path, json.as_bytes()));
        if let Err(e) = written {
            log::warn!("preferences: {}: {e}", path.display());
        }
    }
}

impl eframe::App for Desktop {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        // Docs' Ctrl+O is the Open dialog (WordCraft's would open the file in another window of
        // its own, without our menus); Ctrl+S saves now.
        let (open, save) = ctx.input_mut(|i| {
            (
                i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::O)),
                i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::S)),
            )
        });
        if open {
            self.link.home();
        }
        if save {
            self.link.save_now(&mut self.app);
        }
        self.app.logic(ctx);
        self.link.tick(&mut self.app, ctx);
        for r in self.link.take_requests() {
            match r {
                Request::Open(path) => self.open(path),
                Request::New => spawn(None),
            }
        }
        self.save_prefs();
        let name = self.link.name();
        let title = title(Some(&name));
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        // Closing saves what isn't saved yet. If that fails, the window stays open once, saying
        // why; closing again quits.
        if ctx.input(|i| i.viewport().close_requested()) && self.link.unsaved(&self.app) {
            self.link.save_now(&mut self.app);
            if self.link.unsaved(&self.app) && !self.close_refused {
                self.close_refused = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.app.status("The document couldn't be saved. Close again to quit without saving.");
            }
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // A document dropped on the window opens (here, or in a window of its own); pictures
        // and other files go to WordCraft, which inserts them.
        let dropped = std::mem::take(&mut raw.dropped_files);
        for f in dropped {
            let p = f.path().to_path_buf();
            if p.is_file() && p.extension().and_then(|e| e.to_str()).is_some_and(|e| localfile::DOC_EXTS.contains(&e.to_ascii_lowercase().as_str())) {
                self.link.open_path(p);
            } else {
                raw.dropped_files.push(f);
            }
        }
        self.app.raw_input_hook(ctx, raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.shell.ui(&mut self.app, ui, Some(&self.link));
        self.app.ui(ui);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_prefs();
    }
}
