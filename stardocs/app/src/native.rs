//! The desktop program: the same window as the web app (shell.rs over WordCraft's page) in a
//! native eframe window, drawn with OpenGL through glow as the web build draws with WebGL2.
//!
//! Usage: `stardocs [FILE]`: the document is a file on this computer (localfile.rs), saved back
//! to itself a moment after each change. With no file, a new document is made in
//! `~/Documents/Starbase` on its first save. `stardocs --server-file ID` opens a document on the
//! Starbase server this computer is connected to (File › Connect to a server…, connect.rs;
//! serverfile.rs is its `localfile::Store`), as the Drive in Starbase Sheets launches it.

use std::path::{Path, PathBuf};

use wordcraft_engine::Session;
use wordcraft_ui_egui::{FileDialogAnswer, FileDialogRequest, HostChrome, Services, WordApp};

use crate::localfile::{self, Folders, Link, LocalFile, Request};
use crate::serverfile::ServerDoc;

/// The desktop entry's app id (packaging/linux), also the window's Wayland app id / X11 class.
pub const APP_ID: &str = "io.github.DomeShifter.Starbase.Docs";

const USAGE: &str = "\
stardocs: Starbase Docs, a word processor

USAGE:
  stardocs [FILE]
  stardocs --server-file ID [--name NAME]

  FILE                a document to open (.docx, .odt, .rtf, .txt…); without one, a new
                      document is made in Documents/Starbase when you first type
  --server-file ID    the document ID on the Starbase server this computer is connected
                      to (File › Connect to a server); --name NAME is shown until it opens
  --version           print the version
  -h, --help          print this help
";

/// What the command line asks for.
#[derive(Debug, Default, PartialEq)]
pub struct Args {
    /// Local files: the first opens here, the rest in windows of their own.
    pub files: Vec<PathBuf>,
    /// `--server-file <id> [--name <name>]`: a document on the connected Starbase server (the
    /// id, and the name shown until it opens).
    pub server_file: Option<(String, String)>,
    pub version: bool,
    pub help: bool,
}

/// Reads the command line (without the program's name).
pub fn parse_args(args: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Args, String> {
    let mut out = Args::default();
    let mut name = None;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.to_str() {
            Some("--version") => out.version = true,
            Some("-h" | "--help") => out.help = true,
            Some(flag @ ("--server-file" | "--name")) => {
                let value = it.next().and_then(|v| v.into_string().ok()).filter(|v| !v.is_empty()).ok_or(format!("{flag} needs a value"))?;
                if flag == "--name" {
                    name = Some(value);
                } else {
                    out.server_file = Some((value, String::new()));
                }
            }
            Some(s) if s.starts_with("--") => return Err(format!("unknown option {s}")),
            _ => out.files.push(std::path::absolute(PathBuf::from(&a)).unwrap_or_else(|_| PathBuf::from(&a))),
        }
    }
    if let (Some(f), Some(n)) = (out.server_file.as_mut(), name) {
        f.1 = n;
    }
    Ok(out)
}

/// Parses the command line and runs the window until it closes.
pub fn main() -> std::process::ExitCode {
    env_logger_lite();
    let args = match parse_args(std::env::args_os().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            let _ = std::io::Write::write_fmt(&mut std::io::stderr(), format_args!("stardocs: {e}\n\n{USAGE}"));
            return std::process::ExitCode::from(2);
        }
    };
    if args.version {
        let _ = std::io::Write::write_fmt(&mut std::io::stdout(), format_args!("stardocs {}\n", env!("CARGO_PKG_VERSION")));
        return std::process::ExitCode::SUCCESS;
    }
    if args.help {
        let _ = std::io::Write::write_all(&mut std::io::stdout(), USAGE.as_bytes());
        return std::process::ExitCode::SUCCESS;
    }
    let server_file = args.server_file;
    let mut files = args.files.into_iter();
    // A server document takes this window; local files open in windows of their own.
    let file = if server_file.is_some() { None } else { files.next() };
    for more in files {
        spawn(Some(&more));
    }
    let folders = Folders::of_user().unwrap_or_else(|| Folders { documents: PathBuf::from("Starbase"), data: PathBuf::from(".starbase") });
    if let Err(e) = std::fs::create_dir_all(&folders.documents) {
        log::warn!("{}: {e}", folders.documents.display());
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(title(file.as_ref().and_then(|f| f.file_name()).map(|n| n.to_string_lossy().into_owned()).as_deref()))
        .with_app_id(APP_ID)
        .with_inner_size(FIRST_SIZE)
        .with_min_inner_size(MIN_SIZE)
        .with_drag_and_drop(true);
    if let Some(icon) = icon() {
        viewport = viewport.with_icon(icon);
    }
    // eframe keeps the window's size and place (`persist_window`) for the next start, in
    // ~/.local/share/starbasedocs.
    let options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Glow, centered: true, persist_window: true, ..Default::default() };
    let run = eframe::run_native(
        "Starbase Docs",
        options,
        Box::new(move |cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            // No size kept from an earlier start: the first frames size the window to the screen.
            let kept = cc.storage.and_then(|s| s.get_string(WINDOW_KEY)).is_some();
            Ok(Box::new(Desktop::new(&cc.egui_ctx, file, server_file, folders, !kept)))
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

/// The window's size before the screen's is known, and the smallest it can be (points).
const FIRST_SIZE: [f32; 2] = [1024.0, 720.0];
const MIN_SIZE: [f32; 2] = [640.0, 480.0];
/// eframe's storage key for the window's kept size and place.
const WINDOW_KEY: &str = "window";
/// A first start sizes the window to this share of the screen.
const SCREEN_SHARE: f32 = 0.8;
/// How many frames to wait for the screen's size before leaving the first size as it is.
const SIZE_FRAMES: u32 = 30;

/// The window's size on a first start: 80 % of the screen (`monitor`, points), at least the
/// smallest size and never more than the screen.
pub fn first_size(monitor: egui::Vec2) -> egui::Vec2 {
    (monitor * SCREEN_SHARE).max(egui::Vec2::from(MIN_SIZE)).min(monitor)
}

/// Gives the window our title (`ours`, from [`title`]); `sent` is the one last sent. Call after
/// `WordApp::ui`: WordCraft sends its own ("<name> - WordCraft") from there whenever that changes
/// (a document opened, the first change after a save), and ours, sent after it, is the one the
/// window keeps. Otherwise nothing is sent: a title each frame would keep the window redrawing.
pub fn keep_title(ctx: &egui::Context, ours: &str, sent: &mut String) {
    let theirs = ctx.viewport(|v| v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::Title(t) if t != ours)));
    if theirs || ours != sent.as_str() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(ours.to_string()));
        *sent = ours.to_string();
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
    /// Frames left to size the window to the screen (a first start), 0 once done.
    size_frames: u32,
    /// A close was refused once because the document couldn't be saved: the next one quits.
    close_refused: bool,
    /// The Starbase server this computer is connected to (File › Connect to a server…).
    server: Option<starbase_connect::Server>,
    connect: crate::connect::ConnectDialog,
}

impl Desktop {
    fn new(ctx: &egui::Context, file: Option<PathBuf>, server_file: Option<(String, String)>, folders: Folders, fit_screen: bool) -> Self {
        crate::bundled_fonts::add();
        let stored = prefs_path().and_then(|p| std::fs::read_to_string(p).ok());
        let server = starbase_connect::saved();
        let (link, open) = match (&server_file, &server, &file) {
            (Some((id, name)), Some(s), _) => (Link::new(Box::new(ServerDoc::new(Box::new(s.clone()), id, name)), folders.clone()), true),
            (None, _, Some(f)) => (Link::new(Box::new(LocalFile::open(f.clone())), folders.clone()), true),
            _ => (Link::new(Box::new(LocalFile::new_in(folders.documents.clone())), folders.clone()), false),
        };
        link.set_server(server.as_ref().map(|s| s.url.clone()));
        let services = Services { file_dialog: Some(Box::new(file_dialog)), ..Default::default() };
        let mut app = make_app(&link, services, stored.as_deref());
        if open {
            link.open(&mut app);
        } else if server_file.is_some() {
            app.status("This computer isn't connected to a Starbase server: File › Connect to a server…, then open the document again.");
        }
        ctx.request_repaint();
        Self {
            app,
            shell: crate::shell::Shell::default(),
            link,
            folders,
            prefs: crate::prefs::PrefsSync::new(stored),
            title: String::new(),
            size_frames: if fit_screen { SIZE_FRAMES } else { 0 },
            close_refused: false,
            server,
            connect: crate::connect::ConnectDialog::default(),
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

    /// A first start: once the screen's size is known, 80 % of it, around the window's centre.
    fn fit_screen(&mut self, ctx: &egui::Context) {
        if self.size_frames == 0 {
            return;
        }
        self.size_frames -= 1;
        let (monitor, outer) = ctx.input(|i| (i.viewport().monitor_size, i.viewport().outer_rect));
        let Some(monitor) = monitor.filter(|m| m.x > 0.0 && m.y > 0.0) else { return };
        self.size_frames = 0;
        let size = first_size(monitor);
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        if let Some(outer) = outer {
            let at = outer.center() - size / 2.0;
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(at));
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
                Request::Connect => self.connect.open(),
                Request::Disconnect => {
                    if let Some(s) = self.server.take() {
                        self.connect.disconnect(s);
                    }
                }
            }
        }
        self.save_prefs();
        self.fit_screen(ctx);
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
        let app = &mut self.app;
        if let Some(server) = self.connect.ui(ui.ctx(), &mut |msg| app.status(msg)) {
            self.link.set_server(server.as_ref().map(|s| s.url.clone()));
            self.server = server;
        }
        keep_title(ui.ctx(), &title(Some(&self.link.name())), &mut self.title);
    }

    // Only the window's size and place are kept (`persist_window`), not egui's memory of
    // panels and popups.
    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_prefs();
    }
}
