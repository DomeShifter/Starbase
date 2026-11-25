//! The desktop program: the same window as the web app (shell.rs over DeckCraft's slides) in a
//! native eframe window, drawn with OpenGL through glow as the web build draws with WebGL2.
//! Adapted from StarDocs' native.rs.
//!
//! Usage: `starslides [FILE]`: the presentation is a file on this computer (localfile.rs), saved
//! back to itself a moment after each change. With no file, a new presentation, "Untitled
//! presentation.pptx", is made in `~/Documents/Starbase` on its first save.
//! `starslides --server-file ID` opens a presentation on the Starbase server this computer is
//! connected to (File › Connect to a server…, connect.rs; serverfile.rs is its `Store`), as the
//! Drive in Starbase Sheets launches it.

use std::path::{Path, PathBuf};

use deckcraft_engine::Session;
use deckcraft_ui_egui::{Services, SlideApp};

use crate::localfile::{self, Folders, Link, LocalFile, Request};
use crate::serverfile::ServerDoc;
use crate::shell::Drive as _;

/// The desktop entry's app id (packaging/linux), also the window's Wayland app id / X11 class.
pub const APP_ID: &str = "io.github.DomeShifter.Starbase.Slides";

const USAGE: &str = "\
starslides: Starbase Slides, presentations

USAGE:
  starslides [FILE]
  starslides --new DIR
  starslides --server-file ID [--name NAME]
  starslides --server-new FOLDER

  FILE                a presentation to open (.pptx, .potx, .ppsx); without one, a new
                      presentation is made in Documents/Starbase when you first change it
  --new DIR           a new presentation, made in DIR (instead of Documents/Starbase)
  --server-file ID    the presentation ID on the Starbase server this computer is connected
                      to (File › Connect to a server); --name NAME is shown until it opens
  --server-new FOLDER a new presentation, made on the connected server in FOLDER (the
                      desktop Drive's New presentation; an empty FOLDER is your own)
  --version           print the version
  -h, --help          print this help
";

/// What the command line asks for.
#[derive(Debug, Default, PartialEq)]
pub struct Args {
    /// Local files: the first opens here, the rest in windows of their own.
    pub files: Vec<PathBuf>,
    /// `--server-file <id> [--name <name>]`.
    pub server_file: Option<(String, String)>,
    /// `--new <dir>`: a new presentation made in that folder.
    pub new_in: Option<PathBuf>,
    /// `--server-new <folder>`: a new presentation made on the connected server.
    pub server_new: Option<String>,
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
            Some("--new") => {
                let dir = it.next().filter(|v| !v.is_empty()).ok_or("--new needs a folder")?;
                out.new_in = Some(std::path::absolute(PathBuf::from(&dir)).unwrap_or_else(|_| PathBuf::from(&dir)));
            }
            // The server's folder may be empty: the person's own.
            Some("--server-new") => out.server_new = Some(it.next().and_then(|v| v.into_string().ok()).ok_or("--server-new needs a folder")?),
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
    stderr_logger();
    let args = match parse_args(std::env::args_os().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            starbase_connect::attach_terminal();
            let _ = std::io::Write::write_fmt(&mut std::io::stderr(), format_args!("starslides: {e}\n\n{USAGE}"));
            return std::process::ExitCode::from(2);
        }
    };
    if args.version || args.help {
        starbase_connect::attach_terminal();
    }
    if args.version {
        let _ = std::io::Write::write_fmt(&mut std::io::stdout(), format_args!("starslides {}\n", env!("CARGO_PKG_VERSION")));
        return std::process::ExitCode::SUCCESS;
    }
    if args.help {
        let _ = std::io::Write::write_all(&mut std::io::stdout(), USAGE.as_bytes());
        return std::process::ExitCode::SUCCESS;
    }
    let mut server_file = args.server_file;
    // The desktop Drive's New presentation on a server: made there first, then opened like any.
    let mut start_error = None;
    if let Some(folder) = &args.server_new {
        match new_on_server(folder) {
            Ok(made) => server_file = Some(made),
            Err(e) => start_error = Some(format!("Couldn't make the presentation on the server: {e}")),
        }
    }
    let mut files = args.files.into_iter();
    // A server presentation takes this window; local files open in windows of their own.
    let file = if server_file.is_some() { None } else { files.next() };
    for more in files {
        spawn(Some(&more));
    }
    let mut folders = Folders::of_user().unwrap_or_else(|| Folders { documents: PathBuf::from("Starbase"), data: PathBuf::from(".starbase") });
    // The desktop Drive's New presentation in one of its folders.
    if let Some(dir) = args.new_in {
        folders.documents = dir;
    }
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
    let options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Glow, centered: true, persist_window: true, ..Default::default() };
    let run = eframe::run_native(
        "Starbase Slides",
        options,
        Box::new(move |cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            deckcraft_ui_egui::install_ime_pass_hooks(&cc.egui_ctx);
            let kept = cc.storage.and_then(|s| s.get_string(WINDOW_KEY)).is_some();
            let mut desktop = Desktop::new(&cc.egui_ctx, file, server_file, folders, !kept);
            if let Some(why) = start_error {
                desktop.app.set_status(why);
            }
            Ok(Box::new(desktop))
        }),
    );
    match run {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            let _ = std::io::Write::write_fmt(&mut std::io::stderr(), format_args!("starslides: {e}\n"));
            std::process::ExitCode::FAILURE
        }
    }
}

/// Warnings and errors to standard error (eframe and DeckCraft log through `log`).
fn stderr_logger() {
    struct Stderr;
    impl log::Log for Stderr {
        fn enabled(&self, m: &log::Metadata) -> bool {
            m.level() <= log::Level::Warn
        }
        fn log(&self, r: &log::Record) {
            if self.enabled(r.metadata()) {
                let _ = std::io::Write::write_fmt(&mut std::io::stderr(), format_args!("starslides: {}: {}\n", r.level(), r.args()));
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
const FIRST_SIZE: [f32; 2] = [1280.0, 800.0];
const MIN_SIZE: [f32; 2] = [720.0, 480.0];
/// eframe's storage key for the window's kept size and place.
const WINDOW_KEY: &str = "window";
/// A first start sizes the window to this share of the screen.
const SCREEN_SHARE: f32 = 0.8;
/// How many frames to wait for the screen's size before leaving the first size as it is.
const SIZE_FRAMES: u32 = 30;

/// The window's size on a first start: 80 % of the screen, at least the smallest size and never
/// more than the screen.
pub fn first_size(monitor: egui::Vec2) -> egui::Vec2 {
    (monitor * SCREEN_SHARE).max(egui::Vec2::from(MIN_SIZE)).min(monitor)
}

/// Gives the window our title; `sent` is the one last sent. DeckCraft sends its own whenever it
/// changes; ours, sent after it, is the one the window keeps.
pub fn keep_title(ctx: &egui::Context, ours: &str, sent: &mut String) {
    let theirs = ctx.viewport(|v| v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::Title(t) if t != ours)));
    if theirs || ours != sent.as_str() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(ours.to_string()));
        *sent = ours.to_string();
    }
}

/// "<name> - Starbase Slides", as the web page's tab.
pub fn title(name: Option<&str>) -> String {
    let stem = match name.filter(|n| !n.is_empty()) {
        Some(n) => n.rsplit_once('.').map_or(n, |(s, _)| s),
        None => "Untitled presentation",
    };
    format!("{stem} - Starbase Slides")
}

/// The window icon: app/icons/starslides_logo.svg drawn at 256 px.
fn icon() -> Option<egui::IconData> {
    let svg: &[u8] = include_bytes!("../icons/starslides_logo.svg");
    let img = egui_extras::image::load_svg_bytes_with_size(
        svg,
        egui::SizeHint::Size { width: 256, height: 256, maintain_aspect_ratio: true },
        &Default::default(),
    )
    .map_err(|e| log::warn!("window icon: {e}"))
    .ok()?;
    Some(egui::IconData { rgba: img.as_raw().to_vec(), width: img.width() as u32, height: img.height() as u32 })
}

/// Makes "Untitled presentation.pptx" (an empty presentation) in `folder` on the connected server:
/// its id and name there.
fn new_on_server(folder: &str) -> Result<(String, String), String> {
    let server = starbase_connect::saved().ok_or("this computer isn't connected to a Starbase server")?;
    let mut app = SlideApp::new(Session::new(), Services::default());
    app.run("file.new", serde_json::json!({}))?;
    let doc = app.session.active().map(|d| d.doc.clone()).ok_or("no presentation")?;
    let bytes = deckcraft_engine::cmd::file::save_bytes(&doc, "pptx").map_err(|e| e.to_string())?;
    crate::serverfile::create(&starbase_connect::Renewing::new(server), folder, localfile::NEW_NAME, &bytes)
}

/// Starts another window of this program, on `file` or on a new presentation.
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

/// DeckCraft's own file needs (Insert › Image, Audio, Video, a theme; Export): the system's file
/// dialogs and the disk.
pub fn services() -> Services {
    Services {
        pick_open: Some(Box::new(|purpose: &str| {
            let d = rfd::FileDialog::new();
            let d = match purpose {
                "picture" => d.add_filter("Pictures", &["png", "jpg", "jpeg", "gif", "webp", "bmp"]),
                "audio" => d.add_filter("Audio", &["wav", "mp3", "m4a", "aac", "flac", "ogg", "opus", "aif", "aiff"]),
                "video" => d.add_filter("Video", &["mp4", "m4v", "mov", "webm"]),
                "theme" => d.add_filter("Themes and presentations", &["thmx", "pptx", "potx", "ppsx"]),
                _ => d.add_filter("Presentations", localfile::SLIDE_EXTS),
            };
            d.pick_file().map(|p| p.to_string_lossy().to_string())
        })),
        pick_save: Some(Box::new(|name: &str| rfd::FileDialog::new().set_file_name(name).save_file().map(|p| p.to_string_lossy().to_string()))),
        read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))),
        write: Some(Box::new(|p: &str, b: &[u8]| localfile::write_atomic(Path::new(p), b).map_err(|e| e.to_string()))),
        // One screen as far as we know: Slideshow shows the slides alone; View › Presenter view
        // asks for the presenter window (shell.rs).
        displays: None,
        ..Default::default()
    }
}

/// A window set up as web.rs sets it up (our menus; an empty presentation that the first file
/// opened takes the place of), with the link's saves in place of DeckCraft's own. `new`: a new
/// presentation, named "Untitled presentation.pptx" from the start (never a `.deckcraft`).
pub fn make_app(link: &Link, services: Services, new: bool) -> SlideApp {
    let mut services = services;
    link.connect(&mut services);
    let mut app = SlideApp::new(Session::new(), services).with_chrome(crate::shell::chrome());
    if let Err(e) = app.start_for_host() {
        log::error!("new presentation: {e}");
    }
    if new {
        let blank = app.session.active().map(|d| d.doc.clone()).and_then(|doc| deckcraft_engine::cmd::file::save_bytes(&doc, "pptx").ok());
        if let Some(bytes) = blank {
            app.open_from_host(Some(localfile::NEW_NAME), &bytes);
        }
    }
    app
}

/// Whether the window holds nothing yet: a new presentation no one has changed.
fn is_blank(app: &SlideApp, link: &Link) -> bool {
    link.location().is_none() && !localfile::dirty(app) && link.name().is_empty()
}

struct Desktop {
    app: SlideApp,
    shell: crate::shell::Shell,
    link: Link,
    folders: Folders,
    /// The window title last set.
    title: String,
    /// Frames left to size the window to the screen (a first start), 0 once done.
    size_frames: u32,
    /// A close was refused once because the presentation couldn't be saved: the next one quits.
    close_refused: bool,
    /// The Starbase server this computer is connected to (File › Connect to a server…).
    server: Option<starbase_connect::Server>,
    connect: crate::connect::ConnectDialog,
}

impl Desktop {
    fn new(ctx: &egui::Context, file: Option<PathBuf>, server_file: Option<(String, String)>, folders: Folders, fit_screen: bool) -> Self {
        crate::fonts::add();
        let server = starbase_connect::saved();
        let (link, open) = match (&server_file, &server, &file) {
            (Some((id, name)), Some(s), _) => {
                (Link::new(Box::new(ServerDoc::new(Box::new(starbase_connect::Renewing::new(s.clone())), id, name)), folders.clone()), true)
            }
            (None, _, Some(f)) => (Link::new(Box::new(LocalFile::open(f.clone())), folders.clone()), true),
            _ => (Link::new(Box::new(LocalFile::new_in(folders.documents.clone())), folders.clone()), false),
        };
        link.set_server(server.as_ref().map(|s| s.url.clone()));
        let mut app = make_app(&link, services(), !open);
        if open {
            link.open(&mut app);
        } else if server_file.is_some() {
            app.set_status("This computer isn't connected to a Starbase server: File › Connect to a server…, then open the presentation again.");
        }
        ctx.request_repaint();
        let mode = crate::theme::Mode::parse(&crate::prefs::get(crate::theme::KEY).unwrap_or_default());
        Self {
            app,
            shell: crate::shell::Shell::new(mode),
            link,
            folders,
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
        link.set_server(self.server.as_ref().map(|s| s.url.clone()));
        let mut app = make_app(&link, services(), false);
        if link.open(&mut app) {
            self.app = app;
            self.link = link;
        } else if let Some(why) = link.notice().or_else(|| app.status.as_ref().map(|(m, _)| m.clone())) {
            self.app.set_status(why);
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
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(outer.center() - size / 2.0));
        }
    }
}

/// Ctrl+O and Ctrl+S, taken before DeckCraft sees them: Open is ours (DeckCraft's would open the
/// file as a second presentation in this window, without our saves); Save saves now.
pub fn take_shortcuts(ctx: &egui::Context) -> (bool, bool) {
    use egui::{Key, KeyboardShortcut, Modifiers};
    ctx.input_mut(|i| {
        (
            i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::O)),
            i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::S)),
        )
    })
}

impl eframe::App for Desktop {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let (open, save) = take_shortcuts(ctx);
        if open {
            self.link.home();
        }
        if save {
            self.link.save_now(&mut self.app);
        }
        self.app.logic(ctx);
        self.link.tick(&mut self.app, ctx);
        if starbase_connect::take_changed() {
            self.server = starbase_connect::saved();
            self.link.set_server(self.server.as_ref().map(|s| s.url.clone()));
        }
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
        self.fit_screen(ctx);
        // Closing saves what isn't saved yet. If that fails, the window stays open once, saying
        // why; closing again quits.
        if ctx.input(|i| i.viewport().close_requested()) && self.link.unsaved(&self.app) {
            self.link.save_now(&mut self.app);
            if self.link.unsaved(&self.app) && !self.close_refused {
                self.close_refused = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.app.set_status("The presentation couldn't be saved. Close again to quit without saving.");
            }
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // A presentation dropped on the window opens (here, or in a window of its own); pictures
        // and other files go to DeckCraft, which inserts them.
        let dropped = std::mem::take(&mut raw.dropped_files);
        for f in dropped {
            let p = f.path().to_path_buf();
            if p.is_file() && localfile::is_presentation(&p.to_string_lossy()) {
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
        let pal = self.shell.palette(ui.ctx());
        let app = &mut self.app;
        if let Some(server) = self.connect.ui(ui.ctx(), &pal, &mut |msg| app.set_status(msg)) {
            self.link.set_server(server.as_ref().map(|s| s.url.clone()));
            self.server = server;
        }
        keep_title(ui.ctx(), &title(Some(&self.link.name())), &mut self.title);
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]
    use super::*;

    #[test]
    fn the_command_line() {
        let a = parse_args(["--server-file", "abc", "--name", "Plan.pptx"].map(std::ffi::OsString::from)).unwrap();
        assert_eq!(a.server_file, Some(("abc".into(), "Plan.pptx".into())));
        let b = parse_args(["/tmp/a.pptx", "/tmp/b.pptx"].map(std::ffi::OsString::from)).unwrap();
        assert_eq!(b.files.len(), 2);
        assert!(parse_args(["--nope"].map(std::ffi::OsString::from)).is_err());
        let c = parse_args(["--new", "/tmp/Plans"].map(std::ffi::OsString::from)).unwrap();
        assert_eq!(c.new_in, Some(PathBuf::from("/tmp/Plans")));
        let d = parse_args(["--server-new", ""].map(std::ffi::OsString::from)).unwrap();
        assert_eq!(d.server_new.as_deref(), Some(""), "an empty folder is your own");
        assert!(parse_args(["--server-file"].map(std::ffi::OsString::from)).is_err());
        assert_eq!(title(Some("Plan.pptx")), "Plan - Starbase Slides");
        assert_eq!(title(None), "Untitled presentation - Starbase Slides");
    }
}
