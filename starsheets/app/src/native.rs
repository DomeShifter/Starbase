//! The desktop program `starsheets` (Linux first; docs/research/desktop-plan.md, job D1a).
//! Window setup, the system services and the preferences files are adapted from GridCraft's
//! `apps/gridcraft/src/main.rs` (v0.19.1, MIT OR Apache-2.0, (c) ArtCraft Team and the GridCraft
//! contributors); the frame around the grid is the same as the web app's (web.rs).
//!
//! ```text
//! starsheets [FILE…]     open spreadsheets from disk (each further file in its own window)
//! starsheets --new [DIR] a new spreadsheet, saved in DIR (default: the local Drive folder)
//! starsheets --home      Starbase Drive: the home screen over ~/Documents/Starbase
//! starsheets             the same as --home
//! ```
//!
//! Local mode: no server and no accounts. A spreadsheet saves itself a moment after each change
//! (as on the server); a file changed outside Starbase since it was opened is never overwritten:
//! the save goes to "<name> (conflicted copy)" next to it (localstore.rs, OnDisk). Files that
//! aren't .xlsx (csv, ods, xls…) open as they are and save as an .xlsx next to the original,
//! because files stay .xlsx (docs/decisions.md).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gridcraft_engine::Session;
use gridcraft_ui_egui::{Inbox, Services, SheetApp};

use crate::localstore::{OnDisk, Saved, launch, stem_of};
use crate::store::{has_data, is_doc, xlsx};

/// Quiet time after the last change before saving (ms), as on the web.
const SAVE_AFTER: f64 = 1500.0;
/// A save to the server that failed is tried again 2 s later, then after twice as long each
/// time it fails again, at most every 30 s (ms).
const RETRY_FIRST: f64 = 2_000.0;
const RETRY_MAX: f64 = 30_000.0;
pub const SHEETS_ID: &str = "io.github.DomeShifter.Starbase.Sheets";
pub const DRIVE_ID: &str = "io.github.DomeShifter.Starbase";

/// What the command line asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Start {
    Version,
    Help,
    Home,
    /// A new spreadsheet, saved in this folder.
    New(PathBuf),
    /// Open these files: the first here, the others each in a window of their own.
    Open(Vec<PathBuf>),
    /// A spreadsheet on the connected Starbase server (`--server-file ID [--name NAME]`, from the
    /// Drive; remote.rs).
    Server {
        id: String,
        name: String,
    },
}

pub fn parse_args(args: &[String]) -> Start {
    let mut files = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--version" | "-V" => return Start::Version,
            "--help" | "-h" => return Start::Help,
            "--home" => return Start::Home,
            "--server-file" => {
                let id = args.get(i + 1).cloned().unwrap_or_default();
                let name = args.iter().position(|a| a == "--name").and_then(|n| args.get(n + 1)).cloned().unwrap_or_default();
                return Start::Server { id, name };
            }
            "--new" => {
                let dir = args.get(i + 1).filter(|a| !a.starts_with("--")).map_or_else(crate::localstore::default_root, PathBuf::from);
                return Start::New(dir);
            }
            // A desktop file's field code that wasn't filled in.
            a if a.starts_with("%") => {}
            a if a.starts_with("--") => log::warn!("unknown option {a}"),
            a => files.push(PathBuf::from(a.strip_prefix("file://").unwrap_or(a))),
        }
        i += 1;
    }
    if files.is_empty() { Start::Home } else { Start::Open(files) }
}

pub fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let start = parse_args(&args);
    if matches!(start, Start::Version | Start::Help) {
        starbase_connect::attach_terminal();
    }
    let start = match start {
        Start::Version => {
            println!("Starbase Sheets {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Start::Help => {
            println!(
                "Starbase Sheets {}\n\nUsage:\n  starsheets [FILE…]      open spreadsheets\n  starsheets --new [DIR]  a new spreadsheet, saved in DIR\n  starsheets --home       Starbase Drive (the home screen)\n  starsheets --server-file ID  a spreadsheet on the connected server",
                env!("CARGO_PKG_VERSION")
            );
            return Ok(());
        }
        Start::Open(files) => {
            // Documents go to Starbase Docs, presentations to Starbase Slides; every spreadsheet
            // after the first gets its own window.
            let mut first = None;
            for f in files {
                let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if is_doc(&name) {
                    if let Err(e) = launch(crate::store::program_for(&name), &[f.as_os_str()]) {
                        eprintln!("{e}");
                    }
                } else if first.is_none() {
                    first = Some(f);
                } else if let Err(e) = launch("starsheets", &[f.as_os_str()]) {
                    eprintln!("{e}");
                }
            }
            match first {
                Some(f) => Start::Open(vec![std::path::absolute(&f).unwrap_or(f)]),
                None => return Ok(()),
            }
        }
        s => s,
    };
    let _ = env_logger_lite();
    crate::fonts::install();
    let home = start == Start::Home;
    let (title, app_id, icon) = if home {
        ("Starbase Drive".to_string(), DRIVE_ID, &include_bytes!("../stardrive-touch-icon.png")[..])
    } else {
        ("Starbase Sheets".to_string(), SHEETS_ID, &include_bytes!("../apple-touch-icon.png")[..])
    };
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(&title)
        .with_app_id(app_id)
        // Until the screen's size is known (first frames): then `first_size`, unless the last
        // size and place were remembered (eframe's persistence, per program: Drive and Sheets
        // keep their own, under their app ids).
        .with_inner_size([1180.0, 740.0])
        .with_min_inner_size([640.0, 420.0])
        .with_drag_and_drop(true);
    if let Some(i) = icon_data(icon) {
        viewport = viewport.with_icon(i);
    }
    // Drawn with wgpu (Vulkan, else OpenGL through EGL), as GridCraft is. A computer where wgpu
    // can't start (no working Vulkan or EGL) gets one more try with eframe's OpenGL renderer
    // (glow, also GLX on X11), as Starbase Docs always uses; eframe reuses its event loop for
    // the second run.
    let options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Wgpu, ..Default::default() };
    let glow = eframe::NativeOptions { renderer: eframe::Renderer::Glow, ..options.clone() };
    match eframe::run_native(&title, options, creator(start.clone())) {
        Err(eframe::Error::Wgpu(e)) => {
            log::error!("graphics: wgpu didn't start ({e}); trying OpenGL (glow)");
            eframe::run_native(&title, glow, creator(start))
        }
        result => result,
    }
}

/// Makes the window's app once eframe has a window: the same for either renderer.
fn creator(start: Start) -> eframe::AppCreator<'static> {
    Box::new(move |cc| {
        let mut app = SheetApp::new(Session::new(), services());
        load_prefs(&mut app);
        starsheets_defaults(&mut app);
        // Update checks ask for Starbase's releases, and only when the person turned them on.
        crate::updates::setup(&mut app, starbase_connect::update_check_on());
        gridcraft_ui_egui::spelling::set(crate::spelling::known);
        SheetApp::setup_context_for_language(&cc.egui_ctx, false, app.ui.language);
        crate::theme::install(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let mut files = LocalFiles::default();
        let home = match start {
            Start::Home => Some(crate::home::Home::with_store(crate::remote::backend())),
            Start::Server { id, name } => {
                match crate::remote::RemoteDoc::fetch(&id) {
                    Ok((doc, path)) => {
                        app.open_path(&path.to_string_lossy());
                        // GridCraft has read it: the copy on disk isn't needed (saves go to the server).
                        let _ = std::fs::remove_file(&path);
                        files.pending_remote = Some((path.to_string_lossy().into_owned(), doc));
                    }
                    Err(e) => app.message = Some((if name.is_empty() { "Starbase Sheets".into() } else { name }, e)),
                }
                None
            }
            Start::New(dir) => {
                files.new_folder = dir;
                None
            }
            Start::Open(paths) => {
                if let Some(p) = paths.first() {
                    app.open_path(&p.to_string_lossy());
                }
                None
            }
            _ => None,
        };
        if app.session.documents().is_empty() {
            app.session.new_workbook();
        }
        let remembered = cc.storage.is_some_and(|s| s.get_string("window").is_some());
        let mut shell = NativeShell::new(app, home, files);
        shell.fit_window = if remembered { 0 } else { FIT_FRAMES };
        Ok(Box::new(shell))
    })
}

/// Frames to wait for the screen's size before giving up on sizing a first window to it.
const FIT_FRAMES: u8 = 30;

/// A first window's size: about 80% of the screen, at least 960 x 600 where the screen has
/// room, never bigger than the screen.
fn first_size(monitor: egui::Vec2) -> egui::Vec2 {
    (monitor * 0.8).max(egui::vec2(960.0, 600.0)).min(monitor)
}

/// Log messages to stderr at warning level and above (RUST_LOG-free; GridCraft's own logger
/// with its log files comes later).
fn env_logger_lite() -> Result<(), log::SetLoggerError> {
    struct Stderr;
    impl log::Log for Stderr {
        fn enabled(&self, m: &log::Metadata) -> bool {
            m.level() <= log::Level::Warn
        }
        fn log(&self, r: &log::Record) {
            if self.enabled(r.metadata()) {
                eprintln!("starsheets: {}: {}", r.level(), r.args());
            }
        }
        fn flush(&self) {}
    }
    static LOGGER: Stderr = Stderr;
    log::set_logger(&LOGGER).map(|()| log::set_max_level(log::LevelFilter::Warn))
}

fn icon_data(png_bytes: &[u8]) -> Option<egui::IconData> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(png_bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => buf[..info.buffer_size()].chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        _ => return None,
    };
    Some(egui::IconData { rgba, width: info.width, height: info.height })
}

/// The Sheets behaviour the web app sets too (web.rs), after stored preferences.
fn starsheets_defaults(app: &mut SheetApp) {
    app.ui.system_theme = false;
    app.chrome = gridcraft_ui_egui::Chrome { title_bar: false, file_tab: false, ribbon: false, sheet_tabs: false, questions: false };
    app.ui.animations = false;
    app.ui.sheets_keys = true;
    app.ui.paste_pictures_in_cell = true;
    app.ui.menu_look = Some(crate::shell::MENU_LOOK);
    crate::settings::Prefs::load().apply(app);
    // New files' author (File › Properties): this computer's user, not GridCraft's default name.
    let name = &mut app.session.prefs.user_name;
    if name.trim().is_empty() || *name == gridcraft_engine::Prefs::default().user_name {
        *name = crate::localstore::os_user_name();
    }
}

// ------------------------------------------------------------ system services

fn services() -> Services {
    Services {
        pick_open: Some(Box::new(|| {
            rfd::FileDialog::new()
                .add_filter("Spreadsheets", &["xlsx", "xlsm", "xls", "xlsb", "ods", "csv", "tsv"])
                .pick_file()
                .and_then(|p| p.to_str().map(str::to_string))
        })),
        pick_picture: Some(Box::new(|| {
            rfd::FileDialog::new()
                .add_filter("Pictures (PNG, JPEG)", &["png", "jpg", "jpeg"])
                .pick_file()
                .and_then(|p| p.to_str().map(str::to_string))
        })),
        pick_save: Some(Box::new(|suggested: &str| {
            let path = Path::new(suggested);
            let stem = path.file_stem().and_then(std::ffi::OsStr::to_str).unwrap_or("Untitled spreadsheet").to_string();
            const TYPES: [(&str, &str); 4] =
                [("Excel Workbook (.xlsx)", "xlsx"), ("CSV UTF-8 (.csv)", "csv"), ("Web Page (.html)", "html"), ("PDF (.pdf)", "pdf")];
            let ext = path.extension().and_then(std::ffi::OsStr::to_str).map(str::to_ascii_lowercase).filter(|e| TYPES.iter().any(|(_, x)| x == e));
            let ext = ext.unwrap_or_else(|| "xlsx".to_string());
            let mut dialog = rfd::FileDialog::new().set_file_name(format!("{stem}.{ext}"));
            for (label, x) in TYPES.iter().filter(|(_, x)| *x == ext).chain(TYPES.iter().filter(|(_, x)| *x != ext)) {
                dialog = dialog.add_filter(*label, &[*x]);
            }
            dialog.save_file().and_then(|p| p.to_str().map(str::to_string))
        })),
        open_url: Some(Box::new(|url: &str| open_with_system(url))),
        ..Default::default()
    }
}

/// Opens a web or mail link with the system (http, https and mailto only).
fn open_with_system(target: &str) {
    let gridcraft_engine::links::LinkKind::External(target) = gridcraft_engine::links::classify(target) else {
        log::warn!("refused to open {target:?}: only http, https and mailto links are opened");
        return;
    };
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let mut command = std::process::Command::new(cmd);
    command.arg(target.as_str());
    if let Err(e) = crate::localstore::start_detached(command) {
        log::warn!("{cmd} {target}: {e}");
    }
}

// ------------------------------------------------------------ preferences (GridCraft's files)

/// `sheets` in Starbase's settings folder (`~/.config/starbase`, `%APPDATA%\Starbase`,
/// `~/Library/Application Support/Starbase`), shared with the server link.
fn config_dir() -> Option<PathBuf> {
    Some(starbase_connect::config_dir()?.join("sheets"))
}

fn prefs_enabled() -> bool {
    std::env::var_os("STARBASE_NO_PREFS").is_none()
}

fn load_prefs(app: &mut SheetApp) {
    app.ui.language = gridcraft_ui_egui::i18n::Language::system();
    let Some(dir) = config_dir().filter(|_| prefs_enabled()) else { return };
    if let Ok(b) = std::fs::read(dir.join("ui.json"))
        && let Ok(ui) = serde_json::from_slice::<gridcraft_ui_egui::UiState>(&b)
    {
        let saved = serde_json::from_slice(&b).ok().and_then(|v| gridcraft_ui_egui::i18n::saved_language(&v));
        let language = saved.unwrap_or(app.ui.language);
        app.ui = ui.at_startup();
        app.ui.language = language;
    }
    if let Ok(b) = std::fs::read(dir.join("prefs.json"))
        && let Ok(p) = serde_json::from_slice(&b)
    {
        app.session.prefs = p;
    }
}

fn save_prefs(app: &SheetApp) {
    let Some(dir) = config_dir().filter(|_| prefs_enabled()) else { return };
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(b) = serde_json::to_vec_pretty(&app.ui) {
        let _ = crate::localstore::write_atomic(&dir.join("ui.json"), &b);
    }
    if let Ok(b) = serde_json::to_vec_pretty(&app.session.prefs) {
        let _ = crate::localstore::write_atomic(&dir.join("prefs.json"), &b);
    }
}

// ------------------------------------------------------------ the open spreadsheets' files

/// One open workbook's file on disk, and the saved copy it was last written from.
struct Linked {
    disk: OnDisk,
    saved: usize,
}

/// Keeps the open workbooks in step with their files on disk: saves a moment after each change,
/// never over an outside change, and the file actions of the File menu.
#[derive(Default)]
pub struct LocalFiles {
    links: HashMap<u64, Linked>,
    /// Where a new spreadsheet is saved (`--new DIR`); the local Drive folder by default.
    pub new_folder: PathBuf,
    seen: Option<(u64, usize)>,
    changed_at: f64,
    error: Option<String>,
    /// The workbooks already looked at (linked, or found not to be .xlsx).
    looked: HashMap<u64, ()>,
    /// Workbooks opened from the connected server (remote.rs): saved there, not on disk.
    remote: HashMap<u64, crate::remote::RemoteDoc>,
    /// The one being opened from the server (`--server-file`): the path GridCraft opened it from.
    pub pending_remote: Option<(String, crate::remote::RemoteDoc)>,
    /// Failed saves to the server in a row, for the wait before the next try.
    backoff: Backoff,
    /// A save to the server just failed: wake the window this long after (ms) to try again.
    wake_in: Option<f64>,
}

/// How long to wait before trying a failed save to the server again: [`RETRY_FIRST`] after the
/// first failure, twice as long after each further one, at most [`RETRY_MAX`].
#[derive(Default)]
struct Backoff {
    failures: u32,
}

impl Backoff {
    /// One more failure in a row: the wait before the next try (ms).
    fn failed(&mut self) -> f64 {
        self.failures = self.failures.saturating_add(1);
        (RETRY_FIRST * 2f64.powi(self.failures.min(16) as i32 - 1)).min(RETRY_MAX)
    }

    fn succeeded(&mut self) {
        self.failures = 0;
    }
}

/// Runs the window's logic again after `ms`, even while it is in the background or covered:
/// eframe runs `App::logic` for a hidden window when a repaint is asked for from another
/// thread, whereas a delayed repaint asked for by the window itself waits until it is shown.
fn wake_after(ctx: &egui::Context, ms: f64) {
    let ctx = ctx.clone();
    let wait = std::time::Duration::from_millis(ms.max(0.0) as u64 + 50);
    let spawned = std::thread::Builder::new().name("save-retry".into()).spawn(move || {
        std::thread::sleep(wait);
        ctx.request_repaint();
    });
    if let Err(e) = spawned {
        log::warn!("couldn't start the save retry timer: {e}");
    }
}

impl LocalFiles {
    fn folder(&self) -> PathBuf {
        if self.new_folder.as_os_str().is_empty() { crate::localstore::default_root() } else { self.new_folder.clone() }
    }

    /// The file of the active workbook, if it has one yet.
    pub fn path(&self, app: &SheetApp) -> Option<PathBuf> {
        app.session.active().and_then(|d| self.links.get(&d.uid)).map(|l| l.disk.path.clone())
    }

    /// Links a workbook opened from an .xlsx file to that file; anything else opened (csv, ods,
    /// a new workbook) gets its file at its first save.
    fn look(&mut self, app: &mut SheetApp) {
        let Some(d) = app.session.active_mut() else { return };
        if self.looked.insert(d.uid, ()).is_some() {
            return;
        }
        if self.pending_remote.as_ref().is_some_and(|(p, _)| d.path.as_deref() == Some(p.as_str()))
            && let Some((_, doc)) = self.pending_remote.take()
        {
            d.title = crate::remote::stem(&doc.name);
            d.path = None;
            self.remote.insert(d.uid, doc);
            return;
        }
        if let Some(p) = d.path.clone() {
            let lower = p.to_ascii_lowercase();
            let path = PathBuf::from(&p);
            // The name without its extension, as Sheets shows it ("Budget", not "Budget.csv").
            d.title = stem_of(&path);
            if lower.ends_with(".xlsx") || lower.ends_with(".xlsm") {
                crate::localstore::note_opened(&crate::localstore::data_dir(), &path);
                self.links.insert(d.uid, Linked { disk: OnDisk::open(path), saved: Arc::as_ptr(&d.saved) as usize });
            }
        } else if d.title.strip_prefix("Book").is_some_and(|n| n.chars().all(|c| c.is_ascii_digit())) {
            // A new workbook is "Untitled spreadsheet", as in Sheets (GridCraft calls it Book1).
            d.title = "Untitled spreadsheet".into();
        }
    }

    pub fn logic(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        self.look(app);
        if let Some(req) = ctx.data_mut(|m| m.remove_temp::<String>(egui::Id::unique("starsheets_files_request"))) {
            self.request(app, ctx, &req);
        }
        self.follow_outside_saves(app);
        self.rename_if_needed(app);
        let now = gridcraft_ui_egui::monotonic_ms();
        if let Some(d) = app.session.active() {
            let key = (d.uid, Arc::as_ptr(&d.wb) as usize);
            if self.seen != Some(key) {
                self.seen = Some(key);
                self.changed_at = now;
            }
        }
        let wanted = app.session.active().is_some_and(|d| self.needs_save(d));
        if wanted && app.editor.is_none() {
            let left = SAVE_AFTER - (now - self.changed_at);
            if left > 0.0 {
                ctx.request_repaint_after(std::time::Duration::from_millis(left.min(RETRY_MAX) as u64 + 1));
            } else {
                self.save_active(app);
                // The save status changes ("Saved to …", "Not saved: …"): show it.
                ctx.request_repaint();
            }
        }
        if let Some(ms) = self.wake_in.take() {
            wake_after(ctx, ms);
        }
        use crate::shell::{SaveState, SaveStatus};
        let remote = app.session.active().and_then(|d| self.remote.get(&d.uid));
        if let Some(r) = remote {
            let status = match (&self.error, wanted) {
                (Some(e), _) => SaveStatus::new(SaveState::Problem, format!("Not saved: {e}")),
                (None, true) => SaveStatus::new(SaveState::Saving, "Saving…"),
                (None, false) => SaveStatus::new(SaveState::Saved, format!("Saved to {}", r.host())),
            };
            let tip = format!("All changes are saved on {}", r.url());
            crate::shell::set_save_status(ctx, Some(status));
            ctx.data_mut(|m| m.insert_temp(egui::Id::unique("starsheets_save_status_tip"), tip));
            return;
        }
        let status = match (&self.error, wanted, self.path(app)) {
            (Some(e), _, _) => Some(SaveStatus::new(SaveState::Problem, format!("Not saved: {e}"))),
            (None, true, _) => Some(SaveStatus::new(SaveState::Saving, "Saving…")),
            (None, false, Some(_)) => Some(SaveStatus::new(SaveState::Saved, "Saved to this computer")),
            (None, false, None) => None,
        };
        let tip = match self.path(app) {
            Some(p) => format!("All changes are saved to {}", p.display()),
            None => format!("A new spreadsheet saves itself in {} as soon as you type in it.", self.folder().display()),
        };
        crate::shell::set_save_status(ctx, status);
        ctx.data_mut(|m| m.insert_temp(egui::Id::unique("starsheets_save_status_tip"), tip));
    }

    fn needs_save(&self, d: &gridcraft_engine::DocState) -> bool {
        if self.remote.contains_key(&d.uid) {
            return d.is_dirty();
        }
        has_data(&d.wb) && (d.is_dirty() || !self.links.contains_key(&d.uid))
    }

    /// GridCraft's own Save (Ctrl+S, Download as .xlsx onto the same file) wrote the file: take
    /// that as the version this copy is based on, so the next save isn't seen as a conflict.
    fn follow_outside_saves(&mut self, app: &SheetApp) {
        let Some(d) = app.session.active() else { return };
        let Some(l) = self.links.get_mut(&d.uid) else { return };
        let saved = Arc::as_ptr(&d.saved) as usize;
        if saved != l.saved && !d.is_dirty() {
            l.saved = saved;
            if d.path.as_deref().map(Path::new) == Some(l.disk.path.as_path()) {
                l.disk.refresh();
            }
        }
    }

    /// The title was changed in the title bar: the file follows (a free name in its folder).
    fn rename_if_needed(&mut self, app: &mut SheetApp) {
        let Some(d) = app.session.active_mut() else { return };
        if let Some(r) = self.remote.get_mut(&d.uid) {
            let title = d.title.trim().to_string();
            if !title.is_empty() && title != crate::remote::stem(&r.name) {
                match r.rename(&title) {
                    Ok(()) => d.title = crate::remote::stem(&r.name),
                    Err(e) => {
                        self.error = Some(e);
                        d.title = crate::remote::stem(&r.name);
                    }
                }
            }
            return;
        }
        let Some(l) = self.links.get_mut(&d.uid) else { return };
        let title = d.title.trim().to_string();
        if title.is_empty() || title == stem_of(&l.disk.path) {
            return;
        }
        match l.disk.rename(&title) {
            Ok(()) => {
                d.title = stem_of(&l.disk.path);
                d.path = Some(l.disk.path.to_string_lossy().into_owned());
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// Saves the active workbook now.
    pub fn save_active(&mut self, app: &mut SheetApp) {
        let Some(d) = app.session.active() else { return };
        let (doc, sent) = (d.uid, d.wb.clone());
        // A file from the server is saved in its own format (a .csv stays a .csv).
        let bytes = match self.remote.get(&doc).map_or_else(|| xlsx(&sent, "book.xlsx"), |r| r.bytes(&sent)) {
            Ok(b) => b,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        if let Some(r) = self.remote.get_mut(&doc) {
            match r.save(&bytes) {
                Ok(copy) => {
                    self.error = None;
                    self.backoff.succeeded();
                    if let Some(name) = copy {
                        app.message = Some((
                            "Saved as a copy".into(),
                            format!(
                                "Someone changed this spreadsheet on the server since you opened it, so your changes were saved as \"{name}\" next to it. Nothing is lost."
                            ),
                        ));
                    }
                    if let Some(d) = app.session.active_mut().filter(|d| d.uid == doc) {
                        d.saved = sent;
                        d.title = crate::remote::stem(&r.name);
                    }
                }
                Err(e) => {
                    self.error = Some(e);
                    // Tried again in a while (connected again meanwhile, say), not every frame,
                    // and also while the window is in the background ([`wake_after`]).
                    let wait = self.backoff.failed();
                    self.changed_at = gridcraft_ui_egui::monotonic_ms() + wait - SAVE_AFTER;
                    self.wake_in = Some(wait);
                }
            }
            return;
        }
        if !self.links.contains_key(&doc) {
            // The first save: next to the file it came from (csv, ods…), else in the folder.
            let (dir, stem) = match d.path.as_deref().map(PathBuf::from) {
                Some(p) => (p.parent().map_or_else(|| self.folder(), Path::to_path_buf), d.title.clone()),
                None => (self.folder(), d.title.clone()),
            };
            match OnDisk::create(&dir, &stem) {
                Ok(disk) => {
                    self.links.insert(doc, Linked { disk, saved: 0 });
                }
                Err(e) => {
                    self.error = Some(format!("{}: {e}", dir.display()));
                    return;
                }
            }
        }
        let Some(l) = self.links.get_mut(&doc) else { return };
        match l.disk.save(&bytes) {
            Ok(r) => {
                self.error = None;
                if let Saved::Copy(p) = &r {
                    app.message = Some((
                        "Saved as a copy".into(),
                        format!(
                            "The file was changed outside Starbase since you opened it, so your changes were saved as \"{}\" next to it. Nothing is lost.",
                            p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                        ),
                    ));
                }
                if let Some(d) = app.session.active_mut().filter(|d| d.uid == doc) {
                    d.saved = sent;
                    d.title = stem_of(&l.disk.path);
                    d.path = Some(l.disk.path.to_string_lossy().into_owned());
                    l.saved = Arc::as_ptr(&d.saved) as usize;
                }
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// Saves every workbook with unsaved changes (the window is closing).
    pub fn flush(&mut self, app: &mut SheetApp) {
        let was = app.session.active().map(|d| d.uid);
        let waiting: Vec<usize> = app.session.documents().iter().enumerate().filter(|(_, d)| self.needs_save(d)).map(|(i, _)| i).collect();
        for i in waiting {
            app.session.set_active(i);
            self.save_active(app);
        }
        if let Some(i) = app.session.documents().iter().position(|d| Some(d.uid) == was) {
            app.session.set_active(i);
        }
    }

    /// The File menu's file actions (shell.rs puts them in egui memory).
    fn request(&mut self, app: &mut SheetApp, ctx: &egui::Context, req: &str) {
        let here = self.path(app).and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| self.folder());
        let shown = |r: Result<(), String>, app: &mut SheetApp| {
            if let Err(e) = r {
                app.message = Some(("Starbase".into(), e));
            }
        };
        // A spreadsheet from the server: its own copy, details and bin.
        if let Some(r) = app.session.active().and_then(|d| self.remote.get(&d.uid)) {
            match req {
                "copy" => return shown(r.copy(), app),
                "details" => {
                    let text = format!("Saved on {}, in {}", r.url(), if r.folder.is_empty() { "the top folder" } else { r.folder.as_str() });
                    app.message = Some(("Details".into(), format!("{text}\n\nStarbase Sheets {}", env!("CARGO_PKG_VERSION"))));
                    return;
                }
                "versions" => {
                    app.message = Some(("Version history".into(), format!("See this spreadsheet's versions in the browser, at {}.", r.url())));
                    return;
                }
                "bin" => {
                    match r.bin() {
                        Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                        Err(e) => app.message = Some(("Move to bin".into(), e)),
                    }
                    return;
                }
                _ => {}
            }
        }
        match req {
            "open" => {
                let picked = rfd::FileDialog::new()
                    .set_directory(&here)
                    .add_filter("Spreadsheets", &["xlsx", "xlsm", "xls", "xlsb", "ods", "csv", "tsv"])
                    .add_filter("Documents", &["docx", "odt"])
                    .add_filter("Presentations", &["pptx"])
                    .pick_file();
                let Some(p) = picked else { return };
                let blank = app.session.active().is_none_or(|d| !has_data(&d.wb) && d.path.is_none());
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if blank && !is_doc(&name) {
                    app.open_path(&p.to_string_lossy());
                } else {
                    shown(crate::localstore::open_in_program(&p), app);
                }
            }
            "new" => shown(launch("starsheets", &["--new".as_ref(), here.as_os_str()]), app),
            "home" | "home_tab" => shown(launch("starsheets", &["--home".as_ref()]), app),
            "copy" => {
                let Some(d) = app.session.active() else { return };
                let made = xlsx(&d.wb, "copy.xlsx").and_then(|bytes| {
                    let mut f = OnDisk::create(&here, &format!("Copy of {}", d.title)).map_err(|e| e.to_string())?;
                    f.save(&bytes).map_err(|e| e.to_string())?;
                    launch("starsheets", &[f.path.as_os_str()])
                });
                shown(made, app);
            }
            "details" => {
                let text = match self.path(app) {
                    Some(p) => format!("Saved on this computer in\n{}", p.display()),
                    None => "Not saved yet: a new spreadsheet saves itself as soon as you type in it.".into(),
                };
                app.message = Some(("Details".into(), format!("{text}\n\nStarbase Sheets {}", env!("CARGO_PKG_VERSION"))));
            }
            "versions" => {
                app.message = Some(("Version history".into(), "Files on this computer have no version history yet.".into()));
            }
            "bin" => {
                let Some(uid) = app.session.active().map(|d| d.uid) else { return };
                let Some(l) = self.links.remove(&uid) else {
                    app.message = Some(("Move to bin".into(), "This spreadsheet hasn't been saved yet.".into()));
                    return;
                };
                match trash::delete(&l.disk.path) {
                    Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                    Err(e) => {
                        self.links.insert(uid, l);
                        app.message = Some(("Move to bin".into(), format!("Couldn't move it to the Trash: {e}")));
                    }
                }
            }
            _ => {}
        }
    }
}

// ------------------------------------------------------------ the window

/// The desktop window: the home screen (Starbase Drive) or a spreadsheet, framed as on the web.
struct NativeShell {
    app: SheetApp,
    inbox: Inbox,
    home: Option<crate::home::Home>,
    files: LocalFiles,
    title: String,
    shell: crate::shell::Shell,
    bottom: crate::bottom::Bottom,
    panels: crate::panels::SidePanels,
    find: crate::findreplace::FindReplace,
    messages: crate::messages::Messages,
    link: crate::linkcard::LinkCard,
    autolink: crate::autolink::AutoLink,
    dates: crate::datepicker::DatePicker,
    custom_date: crate::customdate::CustomDate,
    custom_number: crate::customnumber::CustomNumber,
    settings: crate::settings::SettingsWindow,
    input: crate::input::Router,
    notes: crate::notes::Notes,
    calc: crate::calcpreview::CalcPreview,
    merge: crate::merge::MergeGuard,
    spell: crate::spelling::Suggest,
    edit_menu: crate::editmenu::EditMenu,
    error_tip: crate::errortip::ErrorTip,
    /// Frames left to size a first window to the screen (0: done, or the size was remembered).
    fit_window: u8,
}

impl NativeShell {
    fn new(app: SheetApp, home: Option<crate::home::Home>, files: LocalFiles) -> NativeShell {
        NativeShell {
            app,
            inbox: Inbox::default(),
            home,
            files,
            title: String::new(),
            shell: Default::default(),
            bottom: Default::default(),
            panels: Default::default(),
            find: Default::default(),
            messages: Default::default(),
            link: Default::default(),
            autolink: Default::default(),
            dates: Default::default(),
            custom_date: Default::default(),
            custom_number: Default::default(),
            settings: Default::default(),
            input: Default::default(),
            notes: Default::default(),
            calc: Default::default(),
            merge: Default::default(),
            spell: Default::default(),
            edit_menu: Default::default(),
            error_tip: crate::errortip::ErrorTip,
            fit_window: 0,
        }
    }

    fn side_panels(&mut self, ctx: &egui::Context) {
        self.panels.intercept(&mut self.app, ctx);
        self.link.intercept(&mut self.app);
    }

    /// "<file> - Starbase Sheets", or "Starbase Drive".
    fn set_title(&mut self, ctx: &egui::Context) {
        let title = window_title(self.home.is_some(), &self.app);
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

/// "<name> - Starbase Sheets" (the name as the title row shows it), or "Starbase Drive".
fn window_title(home: bool, app: &SheetApp) -> String {
    if home {
        "Starbase Drive".into()
    } else if app.session.active().is_some() {
        format!("{} - Starbase Sheets", crate::shell::header_name(app))
    } else {
        "Starbase Sheets".into()
    }
}

impl eframe::App for NativeShell {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        crate::theme::PAGE.to_normalized_gamma_f32()
    }

    fn persist_egui_memory(&self) -> bool {
        // Only the window's size and place are remembered, not open menus or scroll positions.
        false
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.set_title(ctx);
        if self.fit_window > 0 {
            self.fit_window -= 1;
            if let Some(m) = ctx.input(|i| i.viewport().monitor_size).filter(|m| m.x > 0.0 && m.y > 0.0) {
                let size = first_size(m);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(((m - size) / 2.0).to_pos2()));
                self.fit_window = 0;
            }
        }
        if let Some(home) = &mut self.home {
            home.logic(ctx);
            let dropped: Vec<(String, Vec<u8>)> = std::mem::take(&mut *self.inbox.lock().unwrap_or_else(|e| e.into_inner()));
            for (name, bytes) in dropped {
                home.upload(ctx, name, bytes);
            }
            crate::house::keep(ctx);
            return;
        }
        self.files.logic(&mut self.app, ctx);
        crate::house::keep(ctx);
        self.app.logic(ctx);
        if ctx.input(|i| i.viewport().close_requested()) {
            self.files.flush(&mut self.app);
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // Files dropped on the window: uploaded into the folder shown on the home screen; on a
        // spreadsheet, opened (here if it's blank, else in a window of its own).
        for f in std::mem::take(&mut raw.dropped_files) {
            let p = f.path().to_path_buf();
            if p.as_os_str().is_empty() {
                continue;
            }
            if self.home.is_some() {
                if let Ok(bytes) = std::fs::read(&p) {
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes));
                }
                continue;
            }
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let blank = self.app.session.active().is_none_or(|d| !has_data(&d.wb) && d.path.is_none());
            if blank && !is_doc(&name) {
                self.app.open_path(&p.to_string_lossy());
            } else if let Err(e) = crate::localstore::open_in_program(&p) {
                self.app.message = Some(("Starbase".into(), e));
            }
        }
        if self.home.is_some() {
            return;
        }
        self.input.raw_input(raw);
        self.autolink.saw_input(raw);
        self.app.raw_input_hook(ctx, raw);
    }

    // Keep in step with WebShell::ui (web.rs): the same frame, without the server's parts
    // (live editing, sharing, accounts, the file window).
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(home) = &mut self.home {
            let ctx = ui.ctx().clone();
            home.ui(ui);
            crate::house::style_tooltips(&ctx);
            return;
        }
        let focus_at_start = ui.ctx().memory(|m| m.focused());
        self.shell.ui(&mut self.app, ui);
        self.bottom.ui(&mut self.app, ui);
        let ctx = ui.ctx().clone();
        self.side_panels(&ctx);
        self.panels.ui(&mut self.app, ui);
        self.messages.ui(&mut self.app, &ctx);
        self.spell.before(&ctx);
        self.edit_menu.before(&mut self.app, &ctx);
        self.find.keys(&self.app, &ctx);
        crate::dvpanel::delete_key(&mut self.app);
        self.input.before_grid(&ctx, focus_at_start);
        self.app.ui(ui);
        self.messages.wake(&self.app, &ctx);
        self.input.after_grid(&ctx);
        self.side_panels(&ctx);
        self.autolink.after_frame(&mut self.app);
        self.link.ui(&mut self.app, ui);
        self.link.confirm_ui(&mut self.app, &ctx);
        self.dates.ui(&mut self.app, ui);
        self.notes.intercept(&mut self.app, &ctx);
        self.notes.ui(&mut self.app, ui);
        self.error_tip.ui(&self.app, &ctx);
        self.settings.ui(&mut self.app, &ctx);
        self.custom_date.ui(&mut self.app, &ctx);
        self.custom_number.ui(&mut self.app, &ctx);
        self.find.intercept(&mut self.app);
        self.find.ui(&mut self.app, &ctx);
        self.find.bar_ui(&mut self.app, &ctx);
        self.calc.ui(&self.app, ui);
        self.merge.ui(&mut self.app, &ctx);
        self.panels.protect.warning_ui(&mut self.app, &ctx);
        self.spell.ui(&mut self.app, &ctx);
        self.edit_menu.ui(&mut self.app, &ctx);
        crate::house::style_tooltips(&ctx);
        self.input.end_frame(&ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.home.is_none() {
            self.files.flush(&mut self.app);
        }
        save_prefs(&self.app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn a_first_window_fits_the_screen() {
        assert_eq!(first_size(egui::vec2(2000.0, 1000.0)), egui::vec2(1600.0, 800.0));
        assert_eq!(first_size(egui::vec2(1024.0, 600.0)), egui::vec2(960.0, 600.0), "at least 960 x 600");
        assert_eq!(first_size(egui::vec2(800.0, 500.0)), egui::vec2(800.0, 500.0), "never bigger than the screen");
    }

    #[test]
    fn command_line() {
        assert_eq!(parse_args(&[]), Start::Home);
        assert_eq!(parse_args(&s(&["--home"])), Start::Home);
        assert_eq!(parse_args(&s(&["--version"])), Start::Version);
        assert_eq!(parse_args(&s(&["--new", "/tmp/x"])), Start::New("/tmp/x".into()));
        assert_eq!(parse_args(&s(&["a.xlsx", "file:///b.csv", "%F"])), Start::Open(vec!["a.xlsx".into(), "/b.csv".into()]));
        assert_eq!(
            parse_args(&s(&["--server-file", "ab12", "--name", "Budget.xlsx"])),
            Start::Server { id: "ab12".into(), name: "Budget.xlsx".into() }
        );
    }

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("starbase-native-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn app() -> SheetApp {
        SheetApp::new(Session::new(), Services::default())
    }

    fn a1(app: &SheetApp) -> String {
        let d = app.session.active().unwrap();
        let sh = d.wb.sheet(0).unwrap();
        gridcraft_engine::display::cell_text(&d.wb, sh, gridcraft_engine::core::CellRef { row: 0, col: 0 })
    }

    fn fixture(dir: &Path) -> PathBuf {
        let mut s = Session::new();
        s.new_workbook();
        s.execute("cell.set", json!({"cell": "A1", "input": "Rent"})).unwrap();
        let bytes = xlsx(&s.active().unwrap().wb, "Budget.xlsx").unwrap();
        let p = dir.join("Budget.xlsx");
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn opens_edits_autosaves_and_reopens() {
        let dir = temp("open");
        let path = fixture(&dir);
        let mut app = app();
        app.open_path(&path.to_string_lossy());
        assert_eq!(a1(&app), "Rent");
        let mut files = LocalFiles { new_folder: dir.clone(), ..Default::default() };
        files.look(&mut app);
        assert_eq!(files.path(&app).as_deref(), Some(path.as_path()));
        assert_eq!(app.session.active().unwrap().title, "Budget");
        assert_eq!(crate::shell::header_name(&app), "Budget", "no extension in the title row");
        assert_eq!(window_title(false, &app), "Budget - Starbase Sheets");
        assert_eq!(window_title(true, &app), "Starbase Drive");
        app.run("cell.set", json!({"cell": "A1", "input": "Food"})).unwrap();
        assert!(files.needs_save(app.session.active().unwrap()));
        files.save_active(&mut app);
        assert!(!app.session.active().unwrap().is_dirty());
        let mut again = self::app();
        again.open_path(&path.to_string_lossy());
        assert_eq!(a1(&again), "Food", "the change is in the file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_save_to_the_server_waits_longer_each_time_up_to_30_s() {
        let mut b = Backoff::default();
        let waits: Vec<f64> = (0..7).map(|_| b.failed()).collect();
        assert_eq!(waits, [2_000.0, 4_000.0, 8_000.0, 16_000.0, 30_000.0, 30_000.0, 30_000.0]);
        for _ in 0..100 {
            b.failed();
        }
        assert_eq!(b.failed(), RETRY_MAX, "no overflow after many failures");
        b.succeeded();
        assert_eq!(b.failed(), RETRY_FIRST, "a save that worked starts again from 2 s");
    }

    #[test]
    fn the_retry_timer_wakes_a_window_in_the_background() {
        // The timer asks for a repaint from its own thread once the wait is over.
        use std::sync::atomic::{AtomicBool, Ordering};
        let ctx = egui::Context::default();
        let asked = Arc::new(AtomicBool::new(false));
        let seen = asked.clone();
        ctx.set_request_repaint_callback(move |_| seen.store(true, Ordering::SeqCst));
        wake_after(&ctx, 10.0);
        let start = std::time::Instant::now();
        while !asked.load(Ordering::SeqCst) {
            assert!(start.elapsed() < std::time::Duration::from_secs(5), "the timer never asked for a repaint");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn a_csv_shows_its_name_without_extension_and_saves_next_to_it() {
        let dir = temp("csv");
        let path = dir.join("Data.csv");
        std::fs::write(&path, "Rent,100\n").unwrap();
        let mut app = app();
        app.open_path(&path.to_string_lossy());
        let mut files = LocalFiles { new_folder: dir.clone(), ..Default::default() };
        files.look(&mut app);
        assert_eq!(crate::shell::header_name(&app), "Data");
        assert_eq!(window_title(false, &app), "Data - Starbase Sheets");
        app.run("cell.set", json!({"cell": "A1", "input": "Food"})).unwrap();
        files.save_active(&mut app);
        assert_eq!(files.path(&app), Some(dir.join("Data.xlsx")));
        assert_eq!(crate::shell::header_name(&app), "Data");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_outside_change_gets_a_conflicted_copy() {
        let dir = temp("conflict");
        let path = fixture(&dir);
        let mut app = app();
        app.open_path(&path.to_string_lossy());
        let mut files = LocalFiles { new_folder: dir.clone(), ..Default::default() };
        files.look(&mut app);
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, b"changed by another program").unwrap();
        app.run("cell.set", json!({"cell": "A1", "input": "Mine"})).unwrap();
        files.save_active(&mut app);
        assert_eq!(std::fs::read(&path).unwrap(), b"changed by another program", "never overwritten");
        let copy = dir.join("Budget (conflicted copy).xlsx");
        assert_eq!(files.path(&app).as_deref(), Some(copy.as_path()));
        assert!(app.message.is_some(), "the person is told");
        let mut again = self::app();
        again.open_path(&copy.to_string_lossy());
        assert_eq!(a1(&again), "Mine");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_spreadsheet_saves_in_its_folder_once_it_has_data() {
        let dir = temp("new");
        let mut app = app();
        app.session.new_workbook();
        let mut files = LocalFiles { new_folder: dir.clone(), ..Default::default() };
        files.look(&mut app);
        assert!(!files.needs_save(app.session.active().unwrap()), "nothing in it yet");
        app.run("cell.set", json!({"cell": "A1", "input": "x"})).unwrap();
        files.save_active(&mut app);
        assert_eq!(files.path(&app), Some(dir.join("Untitled spreadsheet.xlsx")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
